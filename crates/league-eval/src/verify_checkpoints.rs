//! Preflight causal verification (`Gate 2`) for Evaluation pre-act checkpoints.
//!
//! `run_verify_checkpoints` replays every pre-act checkpoint pair from Turn 1 in the current
//! engine build, checks format v2 domain-separated save/seal bindings and canonical round-trip byte
//! equality, records each checkpoint's stamped `engine_commit` provenance without gating on commit
//! equality, writes `verified_checkpoints.json` atomically as write-once/read-only, and locks
//! verified checkpoint pairs read-only.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use league_core::EvalHost;
use league_scorer::{
    EVAL_SAVE_FORMAT_VERSION, EVAL_SEAL_FORMAT_VERSION, VERIFIED_CHECKPOINTS_SCHEMA_VERSION,
    VerifiedCheckpointEntry, VerifiedCheckpointsManifest, compute_pre_act_pair_sha256_hex,
    is_valid_sha256_hex, parse_blind_scoring_request_strict, sha256_hex, to_sorted_pretty_json,
};

use crate::error::EvalError;
use crate::serving::{CrateGitShaSource, resolve_crate_git_sha_from};

const EXPECTED_EVAL_SEAT_COUNT: u8 = 4;
const EVAL_SAVE_SHA_DOMAIN_V2: &[u8] = b"league-core/eval-save/v2|";

/// Options for [`run_verify_checkpoints`].
#[derive(Debug, Clone)]
pub struct VerifyCheckpointsOptions {
    /// Directory holding pre-act `<stem>.save.json` / `<stem>.seal` pairs (`--checkpoints <dir>`).
    pub checkpoints_dir: PathBuf,
    /// Write-once output path for `verified_checkpoints.json` (`--out <file>`).
    pub out_path: PathBuf,
    /// Optional anonymized `blind_inputs/` directory to cross-check (`--blind <dir>`).
    pub blind_dir: Option<PathBuf>,
}

/// Summary returned by [`run_verify_checkpoints`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyCheckpointsReport {
    /// Number of checkpoint pairs verified and locked.
    pub verified_count: usize,
    /// SHA-256 hex digest of the written `verified_checkpoints.json` file bytes.
    pub verified_checkpoints_sha256: String,
    /// Verified 40-hex commit SHA of the running `league-eval` build (`engine_build_id`).
    pub engine_build_id: String,
}

enum OuterStemRule {
    None,
    SameStemHex64 { stem: String },
    ContentAddressedVerified,
}

struct ResolvedCheckpointPair {
    save_path: PathBuf,
    seal_path: PathBuf,
    preloaded_save_bytes: Option<Vec<u8>>,
    preloaded_seal_bytes: Option<Vec<u8>>,
    outer_rule: OuterStemRule,
}

fn io_ctx(ctx: &str, path: &Path, err: impl std::fmt::Display) -> EvalError {
    EvalError::Io(format!("{ctx} {}: {err}", path.display()))
}

fn layout_error(message: impl Into<String>) -> EvalError {
    EvalError::CheckpointVerify {
        code: "INVALID_CHECKPOINTS_LAYOUT",
        message: message.into(),
        hint: "pass a directory containing strictly matched `<stem>.save.json` and `<stem>.seal` pairs or content-addressed `<sha256>.save.json` and `<sha256>.seal` pairs",
    }
}

fn collect_checkpoint_pairs(
    checkpoints_dir: &Path,
) -> Result<Vec<ResolvedCheckpointPair>, EvalError> {
    if !checkpoints_dir.is_dir() {
        return Err(EvalError::Usage(format!(
            "`--checkpoints {}` is not an existing directory",
            checkpoints_dir.display()
        )));
    }
    let read_dir = fs::read_dir(checkpoints_dir)
        .map_err(|e| io_ctx("read checkpoints dir", checkpoints_dir, e))?;

    let mut saves: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut seals: BTreeMap<String, PathBuf> = BTreeMap::new();
    for entry_res in read_dir {
        let entry = entry_res.map_err(|e| io_ctx("read entry in", checkpoints_dir, e))?;
        let file_type = entry
            .file_type()
            .map_err(|e| io_ctx("inspect entry type in", checkpoints_dir, e))?;
        let name_os = entry.file_name();
        let Some(name) = name_os.to_str() else {
            return Err(layout_error(
                "non-UTF-8 filename in --checkpoints directory".to_owned(),
            ));
        };
        if !file_type.is_file() {
            return Err(layout_error(format!(
                "disallowed non-file entry `{name}` inside --checkpoints directory"
            )));
        }
        if let Some(stem) = name.strip_suffix(".save.json") {
            if stem.is_empty() {
                return Err(layout_error(format!(
                    "disallowed empty-stem checkpoint file `{name}`"
                )));
            }
            saves.insert(stem.to_owned(), entry.path());
        } else if let Some(stem) = name.strip_suffix(".seal") {
            if stem.is_empty() {
                return Err(layout_error(format!(
                    "disallowed empty-stem checkpoint file `{name}`"
                )));
            }
            seals.insert(stem.to_owned(), entry.path());
        } else {
            return Err(layout_error(format!(
                "disallowed file `{name}` in --checkpoints directory (expected *.save.json or *.seal)"
            )));
        }
    }

    if saves.is_empty() && seals.is_empty() {
        return Err(layout_error(
            "no *.save.json / *.seal checkpoint pairs found in --checkpoints directory",
        ));
    }

    // Case 1: matched `<stem>.save.json` and `<stem>.seal` filenames.
    if saves.len() == seals.len() && saves.keys().eq(seals.keys()) {
        let mut matched = Vec::with_capacity(saves.len());
        for (stem, save_path) in saves {
            let seal_path = seals.remove(&stem).ok_or_else(|| {
                layout_error(format!(
                    "checkpoint `{stem}.save.json` is missing companion `{stem}.seal`"
                ))
            })?;
            let outer_rule = if stem.len() == 64 && stem.bytes().all(|b| b.is_ascii_hexdigit()) {
                OuterStemRule::SameStemHex64 { stem }
            } else {
                OuterStemRule::None
            };
            matched.push(ResolvedCheckpointPair {
                save_path,
                seal_path,
                preloaded_save_bytes: None,
                preloaded_seal_bytes: None,
                outer_rule,
            });
        }
        return Ok(matched);
    }

    // Case 2: content-addressed `<pre_act_save_sha256>.save.json` + `<pre_act_seal_sha256>.seal`
    // exported by `build-blind-inputs --out-checkpoints`.
    if !saves.is_empty()
        && saves.len() == seals.len()
        && saves
            .keys()
            .chain(seals.keys())
            .all(|s| is_valid_sha256_hex(s))
    {
        return pair_content_addressed_checkpoints(saves, seals);
    }

    // Case 3: missing companion or incompatible layout mix.
    for stem in saves.keys() {
        if !seals.contains_key(stem) {
            return Err(layout_error(format!(
                "checkpoint `{stem}.save.json` is missing companion `{stem}.seal`"
            )));
        }
    }
    let missing_save_stem = seals.keys().next().cloned().unwrap_or_default();
    Err(layout_error(format!(
        "checkpoint `{missing_save_stem}.seal` is missing companion `{missing_save_stem}.save.json`"
    )))
}

fn pair_content_addressed_checkpoints(
    saves: BTreeMap<String, PathBuf>,
    seals: BTreeMap<String, PathBuf>,
) -> Result<Vec<ResolvedCheckpointPair>, EvalError> {
    let total_pairs = saves.len();
    let mut saves_by_domain_sha: BTreeMap<String, (PathBuf, Vec<u8>)> = BTreeMap::new();
    for (save_stem, save_path) in saves {
        let save_bytes = fs::read(&save_path).map_err(|e| io_ctx("read", &save_path, e))?;
        let actual_save_sha = sha256_hex(&save_bytes);
        if actual_save_sha != save_stem {
            return Err(EvalError::CheckpointVerify {
                code: "CHECKPOINT_OUTER_PAIR_HASH_MISMATCH",
                message: format!(
                    "content-addressed save `{save_stem}.save.json` has SHA-256 `{actual_save_sha}`"
                ),
                hint: "re-export clean content-addressed checkpoints with `league-eval build-blind-inputs --out-checkpoints`",
            });
        }
        let domain_save_sha = sha256_hex(&[EVAL_SAVE_SHA_DOMAIN_V2, &save_bytes].concat());
        saves_by_domain_sha.insert(domain_save_sha, (save_path, save_bytes));
    }

    let mut verified_seals = Vec::with_capacity(seals.len());
    for (seal_stem, seal_path) in seals {
        let seal_bytes = fs::read(&seal_path).map_err(|e| io_ctx("read", &seal_path, e))?;
        let actual_seal_sha = sha256_hex(&seal_bytes);
        if actual_seal_sha != seal_stem {
            return Err(EvalError::CheckpointVerify {
                code: "CHECKPOINT_OUTER_PAIR_HASH_MISMATCH",
                message: format!(
                    "content-addressed seal `{seal_stem}.seal` has SHA-256 `{actual_seal_sha}`"
                ),
                hint: "re-export clean content-addressed checkpoints with `league-eval build-blind-inputs --out-checkpoints`",
            });
        }
        verified_seals.push((seal_stem, seal_path, seal_bytes));
    }

    let mut resolved = Vec::with_capacity(total_pairs);
    for (seal_stem, seal_path, seal_bytes) in verified_seals {
        let domain_ref = serde_json::from_slice::<serde_json::Value>(&seal_bytes)
            .ok()
            .and_then(|val| {
                val.get("save_sha256")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            });
        if let Some(matched) = domain_ref
            .as_deref()
            .and_then(|d| saves_by_domain_sha.remove(d))
        {
            resolved.push(ResolvedCheckpointPair {
                save_path: matched.0,
                seal_path,
                preloaded_save_bytes: Some(matched.1),
                preloaded_seal_bytes: Some(seal_bytes),
                outer_rule: OuterStemRule::ContentAddressedVerified,
            });
        } else if total_pairs == 1
            && let Some((_, sole_save)) = saves_by_domain_sha.pop_first()
        {
            // Single-pair CAS directories still forward to Gate 2 so v1/corrupt seals report exact inner codes.
            resolved.push(ResolvedCheckpointPair {
                save_path: sole_save.0,
                seal_path,
                preloaded_save_bytes: Some(sole_save.1),
                preloaded_seal_bytes: Some(seal_bytes),
                outer_rule: OuterStemRule::ContentAddressedVerified,
            });
        } else {
            return Err(layout_error(format!(
                "content-addressed seal `{seal_stem}.seal` has no matching `.save.json` in --checkpoints directory"
            )));
        }
    }

    Ok(resolved)
}

fn verify_blind_dir_covered_by_manifest(
    blind_dir: &Path,
    manifest: &VerifiedCheckpointsManifest,
) -> Result<(), EvalError> {
    if !blind_dir.is_dir() {
        return Err(EvalError::VerifiedCheckpointsMismatch(format!(
            "--blind {} is not an existing directory",
            blind_dir.display()
        )));
    }
    let read_dir = fs::read_dir(blind_dir).map_err(|e| io_ctx("read --blind", blind_dir, e))?;
    let mut paths = Vec::new();
    for entry_res in read_dir {
        let entry = entry_res.map_err(|e| io_ctx("read --blind entry in", blind_dir, e))?;
        let ft = entry
            .file_type()
            .map_err(|e| io_ctx("inspect --blind entry in", blind_dir, e))?;
        if ft.is_file() && entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
            paths.push(entry.path());
        }
    }
    paths.sort();
    if paths.is_empty() {
        return Err(EvalError::VerifiedCheckpointsMismatch(format!(
            "no *.json blind input files found in --blind {}",
            blind_dir.display()
        )));
    }
    for path in paths {
        let bytes = fs::read(&path).map_err(|e| io_ctx("read", &path, e))?;
        let req = parse_blind_scoring_request_strict(&bytes)?;
        let Some(entry) = manifest.find_entry(&req.pre_act_pair_sha256) else {
            return Err(EvalError::VerifiedCheckpointsMismatch(format!(
                "blind request `{}` references checkpoint pair `{}` absent from verified_checkpoints.json",
                req.state_id, req.pre_act_pair_sha256
            )));
        };
        if entry.pre_act_save_sha256 != req.pre_act_save_sha256
            || entry.pre_act_seal_sha256 != req.pre_act_seal_sha256
        {
            return Err(EvalError::VerifiedCheckpointsMismatch(format!(
                "blind request `{}` save/seal SHA-256 disagrees with verified checkpoint `{}`",
                req.state_id, req.pre_act_pair_sha256
            )));
        }
    }
    Ok(())
}

fn write_readonly_manifest_atomic(out_path: &Path, json_text: &str) -> Result<(), EvalError> {
    if let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| io_ctx("create parent directory for", out_path, e))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out_path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                EvalError::VerifiedCheckpointsExists(format!(
                    "verified_checkpoints.json already exists at {}; refusing to overwrite",
                    out_path.display()
                ))
            } else {
                io_ctx("create", out_path, e)
            }
        })?;
    if let Err(err) = file
        .write_all(json_text.as_bytes())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(out_path);
        return Err(io_ctx("write", out_path, err));
    }
    drop(file);
    set_path_readonly(out_path)?;
    Ok(())
}

fn set_path_readonly(path: &Path) -> Result<(), EvalError> {
    let mut perms = fs::metadata(path)
        .map_err(|e| io_ctx("stat", path, e))?
        .permissions();
    perms.set_readonly(true);
    fs::set_permissions(path, perms).map_err(|e| io_ctx("chmod readonly", path, e))
}

/// Runs Gate 2 causal + canonical verification on every pre-act checkpoint pair in
/// `opts.checkpoints_dir`, writes write-once read-only `verified_checkpoints.json` to
/// `opts.out_path`, and locks verified `.save.json` / `.seal` files read-only.
pub fn run_verify_checkpoints(
    opts: &VerifyCheckpointsOptions,
    git_sha: &CrateGitShaSource<'_>,
) -> Result<VerifyCheckpointsReport, EvalError> {
    let engine_build_id = resolve_crate_git_sha_from(git_sha)?;

    if opts.out_path.exists() {
        return Err(EvalError::VerifiedCheckpointsExists(format!(
            "verified_checkpoints.json already exists at {}; refusing to overwrite",
            opts.out_path.display()
        )));
    }

    let pairs = collect_checkpoint_pairs(&opts.checkpoints_dir)?;
    let mut entries = Vec::with_capacity(pairs.len());
    let mut seen_pair_hashes = BTreeSet::new();
    let mut verified_files = Vec::with_capacity(pairs.len() * 2);

    for pair in pairs {
        let save_bytes = match pair.preloaded_save_bytes {
            Some(bytes) => bytes,
            None => fs::read(&pair.save_path).map_err(|e| io_ctx("read", &pair.save_path, e))?,
        };
        let seal_bytes = match pair.preloaded_seal_bytes {
            Some(bytes) => bytes,
            None => fs::read(&pair.seal_path).map_err(|e| io_ctx("read", &pair.seal_path, e))?,
        };

        let pair_sha = compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes);
        match pair.outer_rule {
            OuterStemRule::SameStemHex64 { ref stem } if *stem != pair_sha => {
                return Err(EvalError::CheckpointVerify {
                    code: "CHECKPOINT_OUTER_PAIR_HASH_MISMATCH",
                    message: format!(
                        "checkpoint filename stem `{stem}` disagrees with domain-separated pair SHA-256 `{pair_sha}`"
                    ),
                    hint: "re-export clean content-addressed checkpoints with `league-eval build-blind-inputs --out-checkpoints`",
                });
            }
            OuterStemRule::None
            | OuterStemRule::SameStemHex64 { .. }
            | OuterStemRule::ContentAddressedVerified => {}
        }

        let (
            verified_pair_sha,
            save_sha,
            seal_sha,
            engine_commit,
            turn,
            seat_count,
            expected_state_digest,
            turn_chain_digest,
            post_probe_state_digest,
        ) = EvalHost::verify_checkpoint_causal_raw(&save_bytes, &seal_bytes).map_err(
            |(code, message, hint)| EvalError::CheckpointVerify {
                code,
                message,
                hint,
            },
        )?;

        if verified_pair_sha != pair_sha {
            return Err(EvalError::CheckpointVerify {
                code: "CHECKPOINT_OUTER_PAIR_HASH_MISMATCH",
                message: format!(
                    "verifier pair digest `{verified_pair_sha}` != expected `{pair_sha}`"
                ),
                hint: "ensure both `league-eval` and `league-core` agree on the pre-act pair domain prefix",
            });
        }
        if !is_valid_sha256_hex(&save_sha) || !is_valid_sha256_hex(&seal_sha) {
            return Err(EvalError::CheckpointVerify {
                code: "CHECKPOINT_OUTER_PAIR_HASH_MISMATCH",
                message: "malformed SHA-256 returned by checkpoint verifier".to_owned(),
                hint: "rebuild and rerun verify-checkpoints",
            });
        }
        if seat_count != EXPECTED_EVAL_SEAT_COUNT {
            return Err(EvalError::InvalidSeatCount {
                actual: usize::from(seat_count),
            });
        }
        if !seen_pair_hashes.insert(verified_pair_sha.clone()) {
            return Err(EvalError::CheckpointVerify {
                code: "DUPLICATE_CHECKPOINT_PAIR",
                message: format!(
                    "duplicate checkpoint content for pair SHA-256 `{verified_pair_sha}`"
                ),
                hint: "remove duplicate checkpoint pairs before running verify-checkpoints",
            });
        }

        entries.push(VerifiedCheckpointEntry {
            pre_act_pair_sha256: verified_pair_sha,
            pre_act_save_sha256: save_sha,
            pre_act_seal_sha256: seal_sha,
            engine_commit,
            turn,
            seat_count,
            expected_state_digest,
            turn_chain_digest,
            post_probe_state_digest,
        });
        verified_files.push(pair.save_path);
        verified_files.push(pair.seal_path);
    }

    entries.sort_by(|a, b| a.pre_act_pair_sha256.cmp(&b.pre_act_pair_sha256));

    let manifest = VerifiedCheckpointsManifest {
        schema_version: VERIFIED_CHECKPOINTS_SCHEMA_VERSION,
        engine_build_id: engine_build_id.clone(),
        save_format_version: EVAL_SAVE_FORMAT_VERSION,
        seal_format_version: EVAL_SEAL_FORMAT_VERSION,
        entries,
    };
    manifest.validate_for_scorer(&engine_build_id)?;

    if let Some(ref blind_dir) = opts.blind_dir {
        verify_blind_dir_covered_by_manifest(blind_dir, &manifest)?;
    }

    let rendered = to_sorted_pretty_json(&manifest)?;
    let verified_checkpoints_sha256 = sha256_hex(rendered.as_bytes());
    write_readonly_manifest_atomic(&opts.out_path, &rendered)?;

    for path in verified_files {
        set_path_readonly(&path)?;
    }

    Ok(VerifyCheckpointsReport {
        verified_count: manifest.entries.len(),
        verified_checkpoints_sha256,
        engine_build_id,
    })
}

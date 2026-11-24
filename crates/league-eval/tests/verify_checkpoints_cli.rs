//! End-to-end integration tests for `league-eval verify-checkpoints`, freeze/join binding,
//! fail-closed v2/build-ID enforcement, provenance vs compatibility, cross-process round-trip,
//! and readout provenance + scenario randomness scope reporting.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use league_core::{ClubBot, EvalActSubmission, EvalHost, Seat, TableId};
use league_eval::{
    BranchOptions, BuildBlindInputsOptions, CRATE_GIT_SHA, CrateGitShaSource, EvalArm, EvalError,
    FreezeOptions, JoinOptions, JoinedDecisionsFile, NoopTransport, PerArmSummaryFile, PlayOptions,
    SCENARIO_RANDOMNESS_SCOPE_STATEMENT, VerifyCheckpointsOptions, execute_cli_with_git_sha,
    run_branch_with_transport_and_git_sha, run_build_blind_inputs, run_freeze, run_join,
    run_play_with_transport_and_git_sha, run_verify_checkpoints,
};
use league_relay::CONTROL_WORDING_ARM;
use league_scorer::cli::run_cli_with_build_id as run_scorer_cli_with_build_id;
use league_scorer::{
    BlindScoringRequest, EVAL_SAVE_FORMAT_VERSION, EVAL_SEAL_FORMAT_VERSION, ScorerConfig,
    VerifiedCheckpointsManifest, compute_pre_act_pair_sha256_hex, sha256_hex,
    to_sorted_pretty_json,
};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const COMMIT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const COMMIT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

struct TmpDir(PathBuf);

impl TmpDir {
    fn new(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let seq = TMP_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("ml_vc_cli_{label}_{}_{}", std::process::id(), seq));
        clear_readonly_recursive(&dir);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[cfg(unix)]
fn clear_readonly_recursive(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Ok(meta) = fs::symlink_metadata(path) {
        let mode = if meta.is_dir() { 0o755 } else { 0o644 };
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
        if meta.is_dir()
            && let Ok(entries) = fs::read_dir(path)
        {
            for entry in entries.flatten() {
                clear_readonly_recursive(&entry.path());
            }
        }
    }
}

#[cfg(not(unix))]
fn clear_readonly_recursive(path: &Path) {
    if let Ok(meta) = fs::symlink_metadata(path) {
        let mut perms = meta.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        if perms.readonly() {
            perms.set_readonly(false);
            let _ = fs::set_permissions(path, perms);
        }
        if meta.is_dir()
            && let Ok(entries) = fs::read_dir(path)
        {
            for entry in entries.flatten() {
                clear_readonly_recursive(&entry.path());
            }
        }
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        clear_readonly_recursive(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn injected_sha(sha: &'static str) -> CrateGitShaSource<'static> {
    CrateGitShaSource::Injected {
        built_sha: sha,
        live_head: Some(sha),
        is_dirty: Some(false),
    }
}

fn live_or_fallback_sha() -> &'static str {
    if CRATE_GIT_SHA.len() == 40 && CRATE_GIT_SHA.bytes().all(|b| b.is_ascii_hexdigit()) {
        CRATE_GIT_SHA
    } else {
        COMMIT_B
    }
}

fn run_eval_cli(args: &[&str], git_sha: &CrateGitShaSource<'_>) -> (i32, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = execute_cli_with_git_sha(
        args.iter().copied().map(OsString::from),
        &mut out,
        &mut err,
        git_sha,
    );
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

fn collect_bot_submissions(
    host: &EvalHost,
) -> Result<BTreeMap<u8, EvalActSubmission>, Box<dyn std::error::Error>> {
    let mut subs = BTreeMap::new();
    for s in 0..host.seat_count() {
        let obs = host.observe_seat(Seat(s))?;
        let mandate = *host.mandate_for_seat(Seat(s))?;
        let dec = ClubBot::new(host.bot_tie_seed().wrapping_add(u64::from(s)))
            .decide_full_turn(&obs, &mandate);
        subs.insert(
            s,
            EvalActSubmission {
                market_actions: dec.pass_a_actions,
                pass_b_actions: dec.pass_b_actions,
                bundle: dec.bundle,
            },
        );
    }
    Ok(subs)
}

fn build_and_export_cas_checkpoint(
    table_id: u64,
    layout_seed: u64,
    bot_tie_seed: u64,
    engine_commit: &str,
    target_turn: u8,
    raw_dir: &Path,
    cas_dir: &Path,
) -> Result<(String, PathBuf, PathBuf), Box<dyn std::error::Error>> {
    let mut host = EvalHost::from_standard_setup(TableId(table_id), 4, layout_seed, bot_tie_seed)?
        .with_engine_commit(engine_commit)?;
    while host.turn() < target_turn {
        let subs = collect_bot_submissions(&host)?;
        let _ = host.apply_act_submissions(&subs)?;
        let _ = host.sweep_turn()?;
    }
    let (raw_save, raw_seal) = host.write_pre_act_checkpoint(raw_dir)?;
    let save_bytes = fs::read(&raw_save)?;
    let seal_bytes = fs::read(&raw_seal)?;
    let pair_sha = compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes);
    fs::create_dir_all(cas_dir)?;
    let cas_save = cas_dir.join(format!("{pair_sha}.save.json"));
    let cas_seal = cas_dir.join(format!("{pair_sha}.seal"));
    fs::write(&cas_save, &save_bytes)?;
    fs::write(&cas_seal, &seal_bytes)?;
    Ok((pair_sha, cas_save, cas_seal))
}

#[test]
fn verify_checkpoints_cli_writes_readonly_locks_and_refuses_overwrite() -> TestResult {
    let tmp = TmpDir::new("cli_happy")?;
    let cas_dir = tmp.path().join("cas_ckpts");
    let (pair_a, save_a, seal_a) = build_and_export_cas_checkpoint(
        9801,
        7,
        11,
        COMMIT_A,
        2,
        &tmp.path().join("raw_a"),
        &cas_dir,
    )?;
    let (pair_b, save_b, seal_b) = build_and_export_cas_checkpoint(
        9802,
        13,
        19,
        COMMIT_B,
        3,
        &tmp.path().join("raw_b"),
        &cas_dir,
    )?;

    let vc_path = tmp.path().join("nested/verified_checkpoints.json");
    let build_src = injected_sha(COMMIT_B);
    let (code, stdout, stderr) = run_eval_cli(
        &[
            "verify-checkpoints",
            "--checkpoints",
            cas_dir.to_str().ok_or("utf8")?,
            "--out",
            vc_path.to_str().ok_or("utf8")?,
        ],
        &build_src,
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("verified_count=2"));
    assert!(stdout.contains(&format!("engine_build_id={COMMIT_B}")));

    assert!(fs::metadata(&vc_path)?.permissions().readonly());
    assert!(fs::metadata(&save_a)?.permissions().readonly());
    assert!(fs::metadata(&seal_a)?.permissions().readonly());
    assert!(fs::metadata(&save_b)?.permissions().readonly());
    assert!(fs::metadata(&seal_b)?.permissions().readonly());

    let vc_bytes = fs::read(&vc_path)?;
    let manifest: VerifiedCheckpointsManifest = serde_json::from_slice(&vc_bytes)?;
    assert_eq!(manifest.engine_build_id, COMMIT_B);
    assert_eq!(manifest.save_format_version, EVAL_SAVE_FORMAT_VERSION);
    assert_eq!(manifest.seal_format_version, EVAL_SEAL_FORMAT_VERSION);
    assert_eq!(manifest.entries.len(), 2);
    assert_eq!(
        manifest
            .find_entry(&pair_a)
            .ok_or("missing pair_a")?
            .engine_commit,
        COMMIT_A
    );
    assert_eq!(
        manifest
            .find_entry(&pair_b)
            .ok_or("missing pair_b")?
            .engine_commit,
        COMMIT_B
    );

    // Overwrite attempt refuses with VERIFIED_CHECKPOINTS_EXISTS (and supports --error-format json).
    let (ow_code, _, ow_err) = run_eval_cli(
        &[
            "--error-format",
            "json",
            "verify-checkpoints",
            "--checkpoints",
            cas_dir.to_str().ok_or("utf8")?,
            "--out",
            vc_path.to_str().ok_or("utf8")?,
        ],
        &build_src,
    );
    assert_eq!(ow_code, 1);
    assert!(ow_err.contains("VERIFIED_CHECKPOINTS_EXISTS"), "{ow_err}");
    Ok(())
}

#[test]
fn verify_checkpoints_refuses_legacy_v1_flag_and_v1_and_layout_and_build_id_errors() -> TestResult {
    let tmp = TmpDir::new("refusals")?;
    let cas_dir = tmp.path().join("cas_ckpts");
    let (pair_a, save_a, seal_a) = build_and_export_cas_checkpoint(
        9803,
        7,
        29,
        COMMIT_A,
        2,
        &tmp.path().join("raw_a"),
        &cas_dir,
    )?;
    let src_b = injected_sha(COMMIT_B);

    // 1. `--allow-legacy-v1` is rejected as an unknown flag.
    let (legacy_flag_code, _, legacy_flag_err) = run_eval_cli(
        &[
            "verify-checkpoints",
            "--checkpoints",
            cas_dir.to_str().ok_or("utf8")?,
            "--out",
            tmp.path().join("vc_unused.json").to_str().ok_or("utf8")?,
            "--allow-legacy-v1",
        ],
        &src_b,
    );
    assert_eq!(legacy_flag_code, 1);
    assert!(
        legacy_flag_err.contains("INVALID_USAGE") && legacy_flag_err.contains("--allow-legacy-v1"),
        "{legacy_flag_err}"
    );

    // 2. Empty / "unknown" build ID fails closed with INVALID_CRATE_GIT_SHA.
    for bad_build in ["", "unknown", "abc"] {
        let bad_src = CrateGitShaSource::Injected {
            built_sha: bad_build,
            live_head: None,
            is_dirty: Some(false),
        };
        let err = run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: cas_dir.clone(),
                out_path: tmp.path().join("vc_bad_sha.json"),
                blind_dir: None,
            },
            &bad_src,
        )
        .err()
        .ok_or("expected empty/malformed build id refusal")?;
        assert_eq!(err.code(), "INVALID_CRATE_GIT_SHA");
    }

    // 3. v1 checkpoint refused fail-closed with UNSUPPORTED_CHECKPOINT_FORMAT_VERSION.
    let v1_dir = tmp.path().join("v1_ckpts");
    fs::create_dir_all(&v1_dir)?;
    let mut save_val: serde_json::Value = serde_json::from_slice(&fs::read(&save_a)?)?;
    let mut seal_val: serde_json::Value = serde_json::from_slice(&fs::read(&seal_a)?)?;
    save_val["format_version"] = json!(1);
    seal_val["format_version"] = json!(1);
    fs::write(
        v1_dir.join("day-02-pre-act.save.json"),
        serde_json::to_vec_pretty(&save_val)?,
    )?;
    fs::write(
        v1_dir.join("day-02-pre-act.seal"),
        serde_json::to_vec(&seal_val)?,
    )?;
    let (v1_code, _, v1_err) = run_eval_cli(
        &[
            "verify-checkpoints",
            "--checkpoints",
            v1_dir.to_str().ok_or("utf8")?,
            "--out",
            tmp.path().join("vc_v1.json").to_str().ok_or("utf8")?,
        ],
        &src_b,
    );
    assert_eq!(v1_code, 1);
    assert!(
        v1_err.contains("INCOMPATIBLE_ENGINE_OR_SCHEMA_VERSION"),
        "{v1_err}"
    );

    // 4. Layout / CAS filename / duplicate / missing CLI flag errors.
    let (missing_arg_code, _, missing_arg_err) =
        run_eval_cli(&["verify-checkpoints", "--checkpoints", "."], &src_b);
    assert_eq!(missing_arg_code, 1);
    assert!(missing_arg_err.contains("INVALID_USAGE"));

    let missing_dir_err = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: tmp.path().join("no_such_dir"),
            out_path: tmp.path().join("vc_none.json"),
            blind_dir: None,
        },
        &src_b,
    )
    .err()
    .ok_or("expected nonexistent checkpoints dir error")?;
    assert_eq!(missing_dir_err.code(), "INVALID_USAGE");

    let empty_dir = tmp.path().join("empty_ckpts");
    fs::create_dir_all(&empty_dir)?;
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: empty_dir.clone(),
                out_path: tmp.path().join("vc_empty.json"),
                blind_dir: None,
            },
            &src_b,
        )
        .err()
        .ok_or("expected empty dir error")?
        .code(),
        "INVALID_CHECKPOINTS_LAYOUT"
    );

    let subdir_ckpts = tmp.path().join("subdir_ckpts");
    fs::create_dir_all(subdir_ckpts.join("nested"))?;
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: subdir_ckpts,
                out_path: tmp.path().join("vc_sub.json"),
                blind_dir: None,
            },
            &src_b,
        )
        .err()
        .ok_or("expected subdir refusal")?
        .code(),
        "INVALID_CHECKPOINTS_LAYOUT"
    );

    let stray_ckpts = tmp.path().join("stray_ckpts");
    fs::create_dir_all(&stray_ckpts)?;
    fs::write(stray_ckpts.join("notes.txt"), b"x")?;
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: stray_ckpts,
                out_path: tmp.path().join("vc_stray.json"),
                blind_dir: None,
            },
            &src_b,
        )
        .err()
        .ok_or("expected stray file refusal")?
        .code(),
        "INVALID_CHECKPOINTS_LAYOUT"
    );

    let unpaired_ckpts = tmp.path().join("unpaired_ckpts");
    fs::create_dir_all(&unpaired_ckpts)?;
    fs::copy(&save_a, unpaired_ckpts.join(format!("{pair_a}.save.json")))?;
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: unpaired_ckpts,
                out_path: tmp.path().join("vc_unpaired.json"),
                blind_dir: None,
            },
            &src_b,
        )
        .err()
        .ok_or("expected missing companion seal refusal")?
        .code(),
        "INVALID_CHECKPOINTS_LAYOUT"
    );

    let wrong_cas_dir = tmp.path().join("wrong_cas");
    fs::create_dir_all(&wrong_cas_dir)?;
    let fake_stem = "f".repeat(64);
    fs::copy(
        &save_a,
        wrong_cas_dir.join(format!("{fake_stem}.save.json")),
    )?;
    fs::copy(&seal_a, wrong_cas_dir.join(format!("{fake_stem}.seal")))?;
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: wrong_cas_dir,
                out_path: tmp.path().join("vc_cas.json"),
                blind_dir: None,
            },
            &src_b,
        )
        .err()
        .ok_or("expected outer CAS mismatch refusal")?
        .code(),
        "CHECKPOINT_OUTER_PAIR_HASH_MISMATCH"
    );

    let dup_dir = tmp.path().join("dup_ckpts");
    fs::create_dir_all(&dup_dir)?;
    fs::copy(&save_a, dup_dir.join("copy1.save.json"))?;
    fs::copy(&seal_a, dup_dir.join("copy1.seal"))?;
    fs::copy(&save_a, dup_dir.join("copy2.save.json"))?;
    fs::copy(&seal_a, dup_dir.join("copy2.seal"))?;
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: dup_dir,
                out_path: tmp.path().join("vc_dup.json"),
                blind_dir: None,
            },
            &src_b,
        )
        .err()
        .ok_or("expected duplicate checkpoint refusal")?
        .code(),
        "DUPLICATE_CHECKPOINT_PAIR"
    );
    Ok(())
}

#[test]
fn verify_checkpoints_provenance_cross_commit_allows_replay_match_and_refuses_divergence()
-> TestResult {
    let tmp = TmpDir::new("provenance_replay")?;
    let cas_dir = tmp.path().join("cas_ckpts");
    let (pair_sha, save_path, seal_path) = build_and_export_cas_checkpoint(
        9805,
        17,
        31,
        COMMIT_A,
        4,
        &tmp.path().join("raw"),
        &cas_dir,
    )?;

    // Checkpoint written at COMMIT_A verifies cleanly when verifier runs at COMMIT_B.
    let vc_ok = tmp.path().join("vc_ok.json");
    let report = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: cas_dir,
            out_path: vc_ok.clone(),
            blind_dir: None,
        },
        &injected_sha(COMMIT_B),
    )?;
    assert_eq!(report.engine_build_id, COMMIT_B);
    let manifest: VerifiedCheckpointsManifest = serde_json::from_slice(&fs::read(&vc_ok)?)?;
    assert_eq!(
        manifest.find_entry(&pair_sha).ok_or("entry")?.engine_commit,
        COMMIT_A
    );

    // Mutate snapshot state inside a canonically preserved save/seal hash pair (simulating a
    // state divergence relative to the sealed engine state) -> Gate 2 refuses with
    // TIP_STATE_DIGEST_MISMATCH.
    let save_bytes = fs::read(&save_path)?;
    let seal_bytes = fs::read(&seal_path)?;
    let save_val: serde_json::Value = serde_json::from_slice(&save_bytes)?;
    let old_cred = save_val["seats"][0]["cred"]
        .as_i64()
        .ok_or("missing seats[0].cred")?;
    let mut save_str = String::from_utf8(save_bytes.clone())?;
    let seats_pos = save_str.find("\"seats\":").ok_or("missing seats key")?;
    let cred_key = "\"cred\": ";
    let rel_cred = save_str[seats_pos..]
        .find(cred_key)
        .ok_or("missing cred key in seats")?;
    let val_start = seats_pos + rel_cred + cred_key.len();
    let old_cred_str = old_cred.to_string();
    assert_eq!(
        &save_str[val_start..val_start + old_cred_str.len()],
        old_cred_str
    );
    save_str.replace_range(
        val_start..val_start + old_cred_str.len(),
        &(old_cred + 777).to_string(),
    );
    let mutated_save = save_str.into_bytes();

    let old_save_sha = sha256_hex(&[b"league-core/eval-save/v2|" as &[u8], &save_bytes].concat());
    let new_save_sha = sha256_hex(&[b"league-core/eval-save/v2|" as &[u8], &mutated_save].concat());
    let seal_str = String::from_utf8(seal_bytes)?;
    assert!(seal_str.contains(&format!("\"save_sha256\":\"{old_save_sha}\"")));
    let mutated_seal = seal_str
        .replacen(
            &format!("\"save_sha256\":\"{old_save_sha}\""),
            &format!("\"save_sha256\":\"{new_save_sha}\""),
            1,
        )
        .into_bytes();

    let div_dir = tmp.path().join("divergent");
    fs::create_dir_all(&div_dir)?;
    fs::write(div_dir.join("day-04-pre-act.save.json"), &mutated_save)?;
    fs::write(div_dir.join("day-04-pre-act.seal"), &mutated_seal)?;
    let div_err = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: div_dir,
            out_path: tmp.path().join("vc_div.json"),
            blind_dir: None,
        },
        &injected_sha(COMMIT_B),
    )
    .err()
    .ok_or("expected divergent replay refusal")?;
    assert_eq!(div_err.code(), "TIP_STATE_DIGEST_MISMATCH");
    Ok(())
}

#[test]
fn verify_checkpoints_cross_process_round_trip_and_blind_coverage_checks() -> TestResult {
    if std::env::var_os("ML_EVAL_CLI_CHILD").as_deref() == Some(OsStr::new("write_ckpt")) {
        let out_dir = PathBuf::from(
            std::env::var_os("ML_EVAL_CLI_CHILD_DIR").ok_or("ML_EVAL_CLI_CHILD_DIR set")?,
        );
        let mut host =
            EvalHost::from_standard_setup(TableId(9810), 4, 9, 41)?.with_engine_commit(COMMIT_A)?;
        while host.turn() < 3 {
            let subs = collect_bot_submissions(&host)?;
            let _ = host.apply_act_submissions(&subs)?;
            let _ = host.sweep_turn()?;
        }
        host.write_pre_act_checkpoint(&out_dir)?;
        return Ok(());
    }

    let tmp = TmpDir::new("cross_proc_cli")?;
    let ckpts_dir = tmp.path().join("child_ckpts");
    let exe = std::env::current_exe()?;
    let child = Command::new(exe)
        .args([
            "--exact",
            "verify_checkpoints_cross_process_round_trip_and_blind_coverage_checks",
            "--nocapture",
        ])
        .env("ML_EVAL_CLI_CHILD", "write_ckpt")
        .env("ML_EVAL_CLI_CHILD_DIR", &ckpts_dir)
        .stdin(Stdio::null())
        .output()?;
    assert!(
        child.status.success(),
        "child failed: stdout={} stderr={}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );

    let vc_path = tmp.path().join("vc_cross_proc.json");
    let (code, stdout, stderr) = run_eval_cli(
        &[
            "verify-checkpoints",
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--out",
            vc_path.to_str().ok_or("utf8")?,
        ],
        &injected_sha(COMMIT_B),
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("verified_count=1"));

    // `--blind` mismatch checks in `verify-checkpoints`: missing dir, empty dir.
    let err_no_blind = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: ckpts_dir.clone(),
            out_path: tmp.path().join("vc_nb.json"),
            blind_dir: Some(tmp.path().join("missing_blind")),
        },
        &injected_sha(COMMIT_B),
    )
    .err()
    .ok_or("expected missing --blind dir mismatch")?;
    assert_eq!(err_no_blind.code(), "VERIFIED_CHECKPOINTS_MISMATCH");

    let empty_blind = tmp.path().join("empty_blind");
    fs::create_dir_all(&empty_blind)?;
    let err_empty_blind = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: ckpts_dir,
            out_path: tmp.path().join("vc_eb.json"),
            blind_dir: Some(empty_blind),
        },
        &injected_sha(COMMIT_B),
    )
    .err()
    .ok_or("expected empty --blind dir mismatch")?;
    assert_eq!(err_empty_blind.code(), "VERIFIED_CHECKPOINTS_MISMATCH");
    Ok(())
}

fn write_wrapper_and_clubbot_seats(
    dir: &Path,
) -> Result<(PathBuf, PathBuf), Box<dyn std::error::Error>> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("wrappers/v2_seat_brief_2026_11_21_r3.md");
    let wrapper_path = dir.join("wrapper.md");
    fs::write(&wrapper_path, fs::read(&src)?)?;
    let seats_path = dir.join("seats.json");
    let v = json!({
        "seats": [
            { "guild": "Foundry", "backend": "file", "model": "ClubBot" },
            { "guild": "Quarry", "backend": "file", "model": "ClubBot" },
            { "guild": "Harbor", "backend": "file", "model": "ClubBot" },
            { "guild": "Mint", "backend": "file", "model": "ClubBot" }
        ]
    });
    fs::write(&seats_path, serde_json::to_string_pretty(&v)?)?;
    Ok((wrapper_path, seats_path))
}

#[test]
fn freeze_and_join_enforce_verified_checkpoints_and_record_provenance_and_randomness_scope()
-> TestResult {
    let tmp = TmpDir::new("freeze_join_bind")?;
    let (wrapper_path, seats_path) = write_wrapper_and_clubbot_seats(tmp.path())?;
    let transport = NoopTransport;

    // 1. Write the base play + branch fixture at COMMIT_A.
    let src_a = injected_sha(COMMIT_A);
    let play_dir = tmp.path().join("play_run");
    let _play_manifest = run_play_with_transport_and_git_sha(
        &PlayOptions {
            game_dir: play_dir.clone(),
            seats_path: seats_path.clone(),
            wrapper_path: wrapper_path.clone(),
            wrapper_version: None,
            serving_manifest_path: None,
            arm: EvalArm::Control,
            table_id: 9811,
            seed: 5,
            turns: 2,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &src_a,
    )?;

    let branch_dir = tmp.path().join("branch_run");
    let _ = run_branch_with_transport_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(play_dir.join("checkpoints/day-02-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path,
            wrapper_path,
            serving_manifest_path: None,
            arms: vec![EvalArm::Off, EvalArm::Control, EvalArm::On],
            out_dir: branch_dir.clone(),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &transport,
        None,
        &src_a,
    )?;

    let cfg = ScorerConfig {
        scenarios_m: 4,
        ..ScorerConfig::default()
    };
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;

    let blind_dir = tmp.path().join("blind");
    let arm_map_dir = tmp.path().join("arm_map");
    let ckpts_dir = tmp.path().join("ckpts");
    run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![play_dir, branch_dir],
        out_blind_dir: blind_dir.clone(),
        out_arm_map_dir: arm_map_dir.clone(),
        out_checkpoints_dir: Some(ckpts_dir.clone()),
        config_path: cfg_path.clone(),
        scorer_seed: 12,
    })?;

    // 2. Verify, freeze, score, and join at a LATER build id (`live_or_fallback_sha()`, distinct if COMMIT_B or current HEAD != COMMIT_A).
    let build_id_now = live_or_fallback_sha();
    assert_ne!(build_id_now, COMMIT_A);
    let src_now = injected_sha(build_id_now);

    let vc_path = tmp.path().join("verified_checkpoints.json");
    let vc_report = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: ckpts_dir.clone(),
            out_path: vc_path.clone(),
            blind_dir: Some(blind_dir.clone()),
        },
        &src_now,
    )?;
    assert_eq!(vc_report.engine_build_id, build_id_now);

    // `freeze` CLI refuses missing `--verified-checkpoints` with MISSING_VERIFIED_CHECKPOINTS.
    let (freeze_missing_code, _, freeze_missing_err) = run_eval_cli(
        &[
            "freeze",
            "--blind",
            blind_dir.to_str().ok_or("utf8")?,
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--control-wording-arm",
            CONTROL_WORDING_ARM,
            "--out",
            tmp.path().join("f_missing.json").to_str().ok_or("utf8")?,
        ],
        &src_now,
    );
    assert_eq!(freeze_missing_code, 1);
    assert!(
        freeze_missing_err.contains("MISSING_VERIFIED_CHECKPOINTS"),
        "{freeze_missing_err}"
    );

    // `freeze` refuses nonexistent verified_checkpoints file with MISSING_VERIFIED_CHECKPOINTS.
    let missing_vc_err = run_freeze(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path.clone(),
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: tmp.path().join("f_nofile.json"),
            verified_checkpoints_path: tmp.path().join("no_vc.json"),
            scorer_crate_dir: None,
        },
        &src_now,
    )
    .err()
    .ok_or("expected missing vc file error")?;
    assert_eq!(missing_vc_err.code(), "MISSING_VERIFIED_CHECKPOINTS");

    // Valid freeze binds `verified_checkpoints_sha256`.
    let freeze_path = tmp.path().join("scorer_freeze.json");
    let freeze = run_freeze(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path.clone(),
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &src_now,
    )?;
    assert_eq!(
        freeze.verified_checkpoints_sha256,
        vc_report.verified_checkpoints_sha256
    );

    // Score with `build_id_now` + `--freeze` + `--verified-checkpoints`.
    let scores_dir = tmp.path().join("scores");
    let mut s_err = Vec::new();
    let s_code = run_scorer_cli_with_build_id(
        [
            "score",
            "--blind-inputs",
            blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--freeze",
            freeze_path.to_str().ok_or("utf8")?,
            "--verified-checkpoints",
            vc_path.to_str().ok_or("utf8")?,
            "--out",
            scores_dir.to_str().ok_or("utf8")?,
        ],
        build_id_now,
        &mut Vec::new(),
        &mut s_err,
    );
    assert_eq!(s_code, 0, "scorer err: {}", String::from_utf8_lossy(&s_err));

    // `join` CLI refuses without `--verified-checkpoints` -> MISSING_VERIFIED_CHECKPOINTS.
    let (join_no_vc_code, _, join_no_vc_err) = run_eval_cli(
        &[
            "join",
            "--freeze",
            freeze_path.to_str().ok_or("utf8")?,
            "--arm-map",
            arm_map_dir.to_str().ok_or("utf8")?,
            "--scores",
            scores_dir.to_str().ok_or("utf8")?,
            "--out",
            tmp.path().join("j_no_vc").to_str().ok_or("utf8")?,
        ],
        &src_now,
    );
    assert_eq!(join_no_vc_code, 1);
    assert!(
        join_no_vc_err.contains("MISSING_VERIFIED_CHECKPOINTS"),
        "{join_no_vc_err}"
    );

    // `join` refuses a `scorer_freeze.json` lacking `verified_checkpoints_sha256` -> MISSING_VERIFIED_CHECKPOINTS.
    let mut legacy_freeze_val: serde_json::Value =
        serde_json::from_slice(&fs::read(&freeze_path)?)?;
    legacy_freeze_val
        .as_object_mut()
        .ok_or("obj")?
        .remove("verified_checkpoints_sha256");
    let legacy_freeze_path = tmp.path().join("legacy_freeze.json");
    fs::write(
        &legacy_freeze_path,
        serde_json::to_vec_pretty(&legacy_freeze_val)?,
    )?;
    let legacy_join_err = run_join(&JoinOptions {
        freeze_path: legacy_freeze_path,
        arm_map_dir: arm_map_dir.clone(),
        scores_dir: scores_dir.clone(),
        out_dir: tmp.path().join("j_legacy"),
        verified_checkpoints_path: Some(vc_path.clone()),
        blind_dir: None,
        source_game_dirs: Vec::new(),
    })
    .err()
    .ok_or(
        "expected MISSING_VERIFIED_CHECKPOINTS for freeze without verified_checkpoints_sha256",
    )?;
    assert_eq!(legacy_join_err.code(), "MISSING_VERIFIED_CHECKPOINTS");

    // `join` refuses mismatched `--verified-checkpoints` digest -> VERIFIED_CHECKPOINTS_MISMATCH.
    let raw_extra_cas = tmp.path().join("extra_cas");
    let _ = build_and_export_cas_checkpoint(
        9812,
        7,
        19,
        COMMIT_A,
        2,
        &tmp.path().join("raw_extra"),
        &raw_extra_cas,
    )?;
    let other_vc_path = tmp.path().join("other_vc.json");
    let _ = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: raw_extra_cas,
            out_path: other_vc_path.clone(),
            blind_dir: None,
        },
        &src_now,
    )?;
    let mismatch_join_err = run_join(&JoinOptions {
        freeze_path: freeze_path.clone(),
        arm_map_dir: arm_map_dir.clone(),
        scores_dir: scores_dir.clone(),
        out_dir: tmp.path().join("j_mismatch"),
        verified_checkpoints_path: Some(other_vc_path),
        blind_dir: None,
        source_game_dirs: Vec::new(),
    })
    .err()
    .ok_or("expected VERIFIED_CHECKPOINTS_MISMATCH")?;
    assert_eq!(mismatch_join_err.code(), "VERIFIED_CHECKPOINTS_MISMATCH");

    // Verify blind request mismatch in `verify-checkpoints` using a tampered copy of blind_dir.
    let bad_blind = tmp.path().join("bad_blind");
    fs::create_dir_all(&bad_blind)?;
    for entry in fs::read_dir(&blind_dir)? {
        let entry = entry?;
        let mut req: BlindScoringRequest = serde_json::from_slice(&fs::read(entry.path())?)?;
        req.pre_act_pair_sha256 = "f".repeat(64);
        fs::write(
            bad_blind.join(entry.file_name()),
            to_sorted_pretty_json(&req)?,
        )?;
    }
    let bad_blind_vc = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: ckpts_dir,
            out_path: tmp.path().join("vc_bad_blind.json"),
            blind_dir: Some(bad_blind),
        },
        &src_now,
    );
    assert_eq!(
        bad_blind_vc
            .err()
            .ok_or("expected VERIFIED_CHECKPOINTS_MISMATCH on missing blind checkpoint")?
            .code(),
        "VERIFIED_CHECKPOINTS_MISMATCH"
    );

    // Valid join emits provenance + randomness scope fields in both readout files.
    let joined_dir = tmp.path().join("joined_ok");
    let (join_code, join_out, join_err) = run_eval_cli(
        &[
            "join",
            "--freeze",
            freeze_path.to_str().ok_or("utf8")?,
            "--arm-map",
            arm_map_dir.to_str().ok_or("utf8")?,
            "--scores",
            scores_dir.to_str().ok_or("utf8")?,
            "--verified-checkpoints",
            vc_path.to_str().ok_or("utf8")?,
            "--out",
            joined_dir.to_str().ok_or("utf8")?,
        ],
        &src_now,
    );
    assert_eq!(join_code, 0, "join stderr: {join_err}");
    assert!(join_out.contains("states_joined=1 decisions_joined=3"));

    let summary: PerArmSummaryFile =
        serde_json::from_slice(&fs::read(joined_dir.join("per_arm_summary.json"))?)?;
    let decisions: JoinedDecisionsFile =
        serde_json::from_slice(&fs::read(joined_dir.join("joined_decisions.json"))?)?;
    assert_eq!(
        summary.verified_checkpoints_sha256,
        freeze.verified_checkpoints_sha256
    );
    assert_eq!(summary.scorer_engine_build_id, build_id_now);
    assert_eq!(summary.checkpoint_engine_commits, vec![COMMIT_A.to_owned()]);
    assert_eq!(
        summary.scenario_randomness_scope,
        SCENARIO_RANDOMNESS_SCOPE_STATEMENT
    );
    assert!(
        summary.scenario_randomness_scope.contains("shocks")
            && summary.scenario_randomness_scope.contains("honour")
            && summary.scenario_randomness_scope.contains("mandates")
    );
    assert_eq!(
        decisions.verified_checkpoints_sha256,
        freeze.verified_checkpoints_sha256
    );
    assert_eq!(decisions.scorer_engine_build_id, build_id_now);
    assert_eq!(
        decisions.checkpoint_engine_commits,
        vec![COMMIT_A.to_owned()]
    );
    assert_eq!(
        decisions.scenario_randomness_scope,
        SCENARIO_RANDOMNESS_SCOPE_STATEMENT
    );
    for row in &decisions.rows {
        assert_eq!(row.checkpoint_engine_commit, COMMIT_A);
    }
    let _ = EvalError::MissingVerifiedCheckpoints(String::new());
    Ok(())
}

#[test]
fn verify_checkpoints_supports_real_content_addressed_save_and_seal_stems() -> TestResult {
    let tmp = TmpDir::new("real_cas")?;
    let raw_a = tmp.path().join("raw_a");
    let raw_b = tmp.path().join("raw_b");
    let cas_dir = tmp.path().join("real_cas_dir");
    fs::create_dir_all(&cas_dir)?;

    let export_pair = |tid: u64,
                       layout_seed: u64,
                       bot_seed: u64,
                       commit: &str,
                       raw_dir: &Path|
     -> Result<(String, PathBuf, PathBuf), Box<dyn std::error::Error>> {
        let mut host = EvalHost::from_standard_setup(TableId(tid), 4, layout_seed, bot_seed)?
            .with_engine_commit(commit)?;
        let subs = collect_bot_submissions(&host)?;
        let _ = host.apply_act_submissions(&subs)?;
        let _ = host.sweep_turn()?;
        let (s_raw, l_raw) = host.write_pre_act_checkpoint(raw_dir)?;
        let s_bytes = fs::read(&s_raw)?;
        let l_bytes = fs::read(&l_raw)?;
        let s_sha = sha256_hex(&s_bytes);
        let l_sha = sha256_hex(&l_bytes);
        let p_sha = compute_pre_act_pair_sha256_hex(&s_bytes, &l_bytes);
        let s_cas = cas_dir.join(format!("{s_sha}.save.json"));
        let l_cas = cas_dir.join(format!("{l_sha}.seal"));
        fs::write(&s_cas, &s_bytes)?;
        fs::write(&l_cas, &l_bytes)?;
        Ok((p_sha, s_cas, l_cas))
    };

    let (pair_a, save_a, seal_a) = export_pair(9821, 5, 11, COMMIT_A, &raw_a)?;
    let (pair_b, save_b, seal_b) = export_pair(9822, 9, 17, COMMIT_B, &raw_b)?;

    let vc_path = tmp.path().join("vc_real_cas.json");
    let src_now = injected_sha(COMMIT_B);
    let report = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: cas_dir.clone(),
            out_path: vc_path.clone(),
            blind_dir: None,
        },
        &src_now,
    )?;
    assert_eq!(report.verified_count, 2);
    assert!(fs::metadata(&vc_path)?.permissions().readonly());
    assert!(fs::metadata(&save_a)?.permissions().readonly());
    assert!(fs::metadata(&seal_a)?.permissions().readonly());
    assert!(fs::metadata(&save_b)?.permissions().readonly());
    assert!(fs::metadata(&seal_b)?.permissions().readonly());

    let manifest: VerifiedCheckpointsManifest = serde_json::from_slice(&fs::read(&vc_path)?)?;
    assert_eq!(
        manifest
            .find_entry(&pair_a)
            .ok_or("missing pair_a")?
            .engine_commit,
        COMMIT_A
    );
    assert_eq!(
        manifest
            .find_entry(&pair_b)
            .ok_or("missing pair_b")?
            .engine_commit,
        COMMIT_B
    );

    // Wrong 64-hex save filename with distinct valid seal SHA fails closed on outer hash.
    let bad_hash_dir = tmp.path().join("bad_cas_save_hash");
    fs::create_dir_all(&bad_hash_dir)?;
    fs::copy(
        &save_a,
        bad_hash_dir.join(format!("{}.save.json", "e".repeat(64))),
    )?;
    fs::copy(
        &seal_a,
        bad_hash_dir.join(seal_a.file_name().ok_or("seal filename")?),
    )?;
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: bad_hash_dir,
                out_path: tmp.path().join("vc_bad_hash.json"),
                blind_dir: None,
            },
            &src_now,
        )
        .err()
        .ok_or("expected CHECKPOINT_OUTER_PAIR_HASH_MISMATCH")?
        .code(),
        "CHECKPOINT_OUTER_PAIR_HASH_MISMATCH"
    );

    // Unmatched 2-save / 2-seal CAS dir (missing companion save for second seal) -> INVALID_CHECKPOINTS_LAYOUT.
    let raw_c = tmp.path().join("raw_c");
    let orphan_cas_dir = tmp.path().join("orphan_cas_dir");
    fs::create_dir_all(&orphan_cas_dir)?;
    let mut host_c =
        EvalHost::from_standard_setup(TableId(9823), 4, 15, 23)?.with_engine_commit(COMMIT_A)?;
    let subs_c = collect_bot_submissions(&host_c)?;
    let _ = host_c.apply_act_submissions(&subs_c)?;
    let _ = host_c.sweep_turn()?;
    let (_, l_raw_c) = host_c.write_pre_act_checkpoint(&raw_c)?;
    let l_bytes_c = fs::read(&l_raw_c)?;
    let l_sha_c = sha256_hex(&l_bytes_c);
    fs::copy(
        &save_a,
        orphan_cas_dir.join(save_a.file_name().ok_or("save_a")?),
    )?;
    fs::copy(
        &save_b,
        orphan_cas_dir.join(save_b.file_name().ok_or("save_b")?),
    )?;
    fs::copy(
        &seal_a,
        orphan_cas_dir.join(seal_a.file_name().ok_or("seal_a")?),
    )?;
    fs::write(orphan_cas_dir.join(format!("{l_sha_c}.seal")), &l_bytes_c)?;
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: orphan_cas_dir,
                out_path: tmp.path().join("vc_orphan_cas.json"),
                blind_dir: None,
            },
            &src_now,
        )
        .err()
        .ok_or("expected INVALID_CHECKPOINTS_LAYOUT")?
        .code(),
        "INVALID_CHECKPOINTS_LAYOUT"
    );

    Ok(())
}

#[test]
fn verify_checkpoints_and_join_cover_remaining_layout_and_manifest_error_branches() -> TestResult {
    let tmp = TmpDir::new("err_branches")?;
    let (wrapper_path, seats_path) = write_wrapper_and_clubbot_seats(tmp.path())?;
    let src_a = injected_sha(COMMIT_A);

    // 1. Play + build-blind-inputs fixture (reused across freeze/join/blind manifest refusal tests).
    let play_dir = tmp.path().join("play_src");
    let _ = run_play_with_transport_and_git_sha(
        &PlayOptions {
            game_dir: play_dir.clone(),
            seats_path,
            wrapper_path,
            wrapper_version: None,
            serving_manifest_path: None,
            arm: EvalArm::Off,
            table_id: 9831,
            seed: 9,
            turns: 2,
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &NoopTransport,
        None,
        &src_a,
    )?;

    let save_p = play_dir.join("checkpoints/day-02-pre-act.save.json");
    let seal_p = play_dir.join("checkpoints/day-02-pre-act.seal");
    let save_bytes = fs::read(&save_p)?;
    let seal_bytes = fs::read(&seal_p)?;
    let save_sha = sha256_hex(&save_bytes);
    let _pair_sha = compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes);

    // Empty stems `.save.json` / `.seal`, Case-3 unmatched `.seal`, CAS bad seal hash, CLI usage.
    let empty_save_stem = tmp.path().join("empty_save_stem");
    fs::create_dir_all(&empty_save_stem)?;
    fs::write(empty_save_stem.join(".save.json"), b"{}")?;
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: empty_save_stem,
                out_path: tmp.path().join("vc_es1.json"),
                blind_dir: None,
            },
            &src_a,
        )
        .err()
        .ok_or("empty save stem")?
        .code(),
        "INVALID_CHECKPOINTS_LAYOUT"
    );

    let empty_seal_stem = tmp.path().join("empty_seal_stem");
    fs::create_dir_all(&empty_seal_stem)?;
    fs::write(empty_seal_stem.join(".seal"), b"{}")?;
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: empty_seal_stem,
                out_path: tmp.path().join("vc_es2.json"),
                blind_dir: None,
            },
            &src_a,
        )
        .err()
        .ok_or("empty seal stem")?
        .code(),
        "INVALID_CHECKPOINTS_LAYOUT"
    );

    let orphan_seal_dir = tmp.path().join("orphan_seal_dir");
    fs::create_dir_all(&orphan_seal_dir)?;
    fs::copy(&seal_p, orphan_seal_dir.join("day-02-pre-act.seal"))?;
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: orphan_seal_dir,
                out_path: tmp.path().join("vc_os.json"),
                blind_dir: None,
            },
            &src_a,
        )
        .err()
        .ok_or("unmatched seal")?
        .code(),
        "INVALID_CHECKPOINTS_LAYOUT"
    );

    let bad_cas_seal_dir = tmp.path().join("bad_cas_seal");
    fs::create_dir_all(&bad_cas_seal_dir)?;
    fs::write(
        bad_cas_seal_dir.join(format!("{save_sha}.save.json")),
        &save_bytes,
    )?;
    fs::write(
        bad_cas_seal_dir.join(format!("{}.seal", "e".repeat(64))),
        &seal_bytes,
    )?;
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: bad_cas_seal_dir,
                out_path: tmp.path().join("vc_bcs.json"),
                blind_dir: None,
            },
            &src_a,
        )
        .err()
        .ok_or("bad cas seal hash")?
        .code(),
        "CHECKPOINT_OUTER_PAIR_HASH_MISMATCH"
    );

    let (unk_code, _, unk_err) = run_eval_cli(
        &[
            "verify-checkpoints",
            "--checkpoints",
            ".",
            "--out",
            "x",
            "--unknown-flag",
        ],
        &src_a,
    );
    assert_eq!(unk_code, 1);
    assert!(unk_err.contains("INVALID_USAGE"), "{unk_err}");

    let (no_ckpts_code, _, no_ckpts_err) =
        run_eval_cli(&["verify-checkpoints", "--out", "x"], &src_a);
    assert_eq!(no_ckpts_code, 1);
    assert!(no_ckpts_err.contains("INVALID_USAGE"), "{no_ckpts_err}");

    // 2. Clean verify + direct overwrite refusal + freeze/join verified_checkpoints mismatch branches.
    let raw_ckpts = tmp.path().join("ckpts_clean");
    fs::create_dir_all(&raw_ckpts)?;
    fs::write(raw_ckpts.join("day-02-pre-act.save.json"), &save_bytes)?;
    fs::write(raw_ckpts.join("day-02-pre-act.seal"), &seal_bytes)?;
    let missing_blind_err = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: raw_ckpts.clone(),
            out_path: tmp.path().join("vc_mb.json"),
            blind_dir: Some(tmp.path().join("no_such_blind")),
        },
        &src_a,
    )
    .err()
    .ok_or("nonexistent blind dir")?;
    assert_eq!(missing_blind_err.code(), "VERIFIED_CHECKPOINTS_MISMATCH");

    let vc_path = tmp.path().join("vc_clean.json");
    let _ = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: raw_ckpts.clone(),
            out_path: vc_path.clone(),
            blind_dir: None,
        },
        &src_a,
    )?;
    let exists_err = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: raw_ckpts,
            out_path: vc_path.clone(),
            blind_dir: None,
        },
        &src_a,
    )
    .err()
    .ok_or("expected VERIFIED_CHECKPOINTS_EXISTS")?;
    assert_eq!(exists_err.code(), "VERIFIED_CHECKPOINTS_EXISTS");

    // Blind request save/seal hash mismatch with matching pair_sha -> VERIFIED_CHECKPOINTS_MISMATCH.
    let cfg = ScorerConfig {
        scenarios_m: 4,
        ..ScorerConfig::default()
    };
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;
    let (blind_dir, arm_map_dir, ckpts_cas_dir) = (
        tmp.path().join("blind"),
        tmp.path().join("arm_map"),
        tmp.path().join("ckpts_cas"),
    );
    let ws2 = tmp.path().join("ws2");
    fs::create_dir_all(&ws2)?;
    let (wrapper2, seats2) = write_wrapper_and_clubbot_seats(&ws2)?;
    let branch_dir = tmp.path().join("branch_src");
    let _ = run_branch_with_transport_and_git_sha(
        &BranchOptions {
            checkpoint_prefix: Some(play_dir.join("checkpoints/day-02-pre-act")),
            list_path: None,
            focal_seat: Some("Foundry".to_owned()),
            seats_path: seats2,
            wrapper_path: wrapper2,
            serving_manifest_path: None,
            arms: vec![EvalArm::Off, EvalArm::Control, EvalArm::On],
            out_dir: branch_dir.clone(),
            memory_mode: league_relay::MemoryMode::Snapshot,
            live: false,
            cap_credits: None,
            cap_gpu_secs: None,
            allowed_vm_hosts: Vec::new(),
        },
        &NoopTransport,
        None,
        &src_a,
    )?;
    run_build_blind_inputs(&BuildBlindInputsOptions {
        run_dirs: vec![play_dir, branch_dir],
        out_blind_dir: blind_dir.clone(),
        out_arm_map_dir: arm_map_dir.clone(),
        out_checkpoints_dir: Some(ckpts_cas_dir.clone()),
        config_path: cfg_path.clone(),
        scorer_seed: 19,
    })?;

    let vc_cas = tmp.path().join("vc_cas.json");
    let _ = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: ckpts_cas_dir.clone(),
            out_path: vc_cas.clone(),
            blind_dir: Some(blind_dir.clone()),
        },
        &src_a,
    )?;

    let bad_save_blind = tmp.path().join("bad_save_blind");
    fs::create_dir_all(&bad_save_blind)?;
    for entry in fs::read_dir(&blind_dir)? {
        let entry = entry?;
        let mut req: BlindScoringRequest = serde_json::from_slice(&fs::read(entry.path())?)?;
        req.pre_act_save_sha256 = "d".repeat(64);
        fs::write(
            bad_save_blind.join(entry.file_name()),
            to_sorted_pretty_json(&req)?,
        )?;
    }
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: ckpts_cas_dir.clone(),
                out_path: tmp.path().join("vc_bsb.json"),
                blind_dir: Some(bad_save_blind.clone()),
            },
            &src_a,
        )
        .err()
        .ok_or("blind save sha mismatch")?
        .code(),
        "VERIFIED_CHECKPOINTS_MISMATCH"
    );

    // Freeze errors: corrupt vc JSON, wrong build_id in vc, missing pair in vc, disagreeing save_sha in blind.
    let bad_vc_json = tmp.path().join("bad_vc.json");
    fs::write(&bad_vc_json, b"{not json")?;
    assert_eq!(
        run_freeze(
            &FreezeOptions {
                blind_dir: blind_dir.clone(),
                config_path: cfg_path.clone(),
                control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
                out_path: tmp.path().join("f1.json"),
                verified_checkpoints_path: bad_vc_json,
                scorer_crate_dir: None,
            },
            &src_a,
        )
        .err()
        .ok_or("corrupt vc json")?
        .code(),
        "VERIFIED_CHECKPOINTS_MISMATCH"
    );

    assert_eq!(
        run_freeze(
            &FreezeOptions {
                blind_dir: blind_dir.clone(),
                config_path: cfg_path.clone(),
                control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
                out_path: tmp.path().join("f2.json"),
                verified_checkpoints_path: vc_cas.clone(),
                scorer_crate_dir: None,
            },
            &injected_sha(COMMIT_B),
        )
        .err()
        .ok_or("vc build id mismatch")?
        .code(),
        "VERIFIED_CHECKPOINTS_MISMATCH"
    );

    let alt_pair_blind = tmp.path().join("alt_pair_blind");
    fs::create_dir_all(&alt_pair_blind)?;
    for entry in fs::read_dir(&blind_dir)? {
        let entry = entry?;
        let mut req: BlindScoringRequest = serde_json::from_slice(&fs::read(entry.path())?)?;
        req.pre_act_pair_sha256 = "e".repeat(64);
        fs::write(
            alt_pair_blind.join(entry.file_name()),
            to_sorted_pretty_json(&req)?,
        )?;
    }
    for bd in [&alt_pair_blind, &bad_save_blind] {
        assert_eq!(
            run_freeze(
                &FreezeOptions {
                    blind_dir: bd.clone(),
                    config_path: cfg_path.clone(),
                    control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
                    out_path: tmp.path().join("f3.json"),
                    verified_checkpoints_path: vc_cas.clone(),
                    scorer_crate_dir: None,
                },
                &src_a,
            )
            .err()
            .ok_or("freeze vc blind mismatch")?
            .code(),
            "VERIFIED_CHECKPOINTS_MISMATCH"
        );
    }

    // Clean freeze + score + join checks (None vc -> MISSING_VERIFIED_CHECKPOINTS, tampered arm_map -> VERIFIED_CHECKPOINTS_MISMATCH).
    let freeze_path = tmp.path().join("freeze_ok.json");
    let _ = run_freeze(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path.clone(),
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_cas.clone(),
            scorer_crate_dir: None,
        },
        &src_a,
    )?;
    let scores_dir = tmp.path().join("scores_ok");
    let sc_code = run_scorer_cli_with_build_id(
        [
            "score",
            "--blind-inputs",
            blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            ckpts_cas_dir.to_str().ok_or("utf8")?,
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--freeze",
            freeze_path.to_str().ok_or("utf8")?,
            "--verified-checkpoints",
            vc_cas.to_str().ok_or("utf8")?,
            "--out",
            scores_dir.to_str().ok_or("utf8")?,
        ],
        COMMIT_A,
        &mut Vec::new(),
        &mut Vec::new(),
    );
    assert_eq!(sc_code, 0);

    let none_vc_err = run_join(&JoinOptions {
        freeze_path: freeze_path.clone(),
        arm_map_dir: arm_map_dir.clone(),
        scores_dir: scores_dir.clone(),
        out_dir: tmp.path().join("j_none"),
        verified_checkpoints_path: None,
        blind_dir: None,
        source_game_dirs: Vec::new(),
    })
    .err()
    .ok_or("join with verified_checkpoints_path=None")?;
    assert_eq!(none_vc_err.code(), "MISSING_VERIFIED_CHECKPOINTS");

    for (tag, field, val) in [
        ("missing_pair", "pre_act_pair_sha256", "c".repeat(64)),
        ("diff_save", "pre_act_save_sha256", "d".repeat(64)),
    ] {
        let tampered_arm = tmp.path().join(format!("arm_map_{tag}"));
        fs::create_dir_all(&tampered_arm)?;
        for entry in fs::read_dir(&arm_map_dir)? {
            let entry = entry?;
            let mut v: serde_json::Value = serde_json::from_slice(&fs::read(entry.path())?)?;
            v[field] = json!(val);
            fs::write(
                tampered_arm.join(entry.file_name()),
                serde_json::to_vec_pretty(&v)?,
            )?;
        }
        assert_eq!(
            run_join(&JoinOptions {
                freeze_path: freeze_path.clone(),
                arm_map_dir: tampered_arm,
                scores_dir: scores_dir.clone(),
                out_dir: tmp.path().join(format!("j_{tag}")),
                verified_checkpoints_path: Some(vc_cas.clone()),
                blind_dir: None,
                source_game_dirs: Vec::new(),
            })
            .err()
            .ok_or("expected VERIFIED_CHECKPOINTS_MISMATCH on tampered arm_map")?
            .code(),
            "VERIFIED_CHECKPOINTS_MISMATCH"
        );
    }

    // Exercise EvalError Display / CLI error text for the three new variants cleanly,
    // plus JSON error-format serialization on VerifiedCheckpointsMismatch (ToStructuredError),
    // single-pair v1 CAS layout forwarding, and unwritable `--out` parent path IO error.
    for e in [
        EvalError::VerifiedCheckpointsExists("exists".to_owned()),
        EvalError::MissingVerifiedCheckpoints("missing".to_owned()),
        EvalError::VerifiedCheckpointsMismatch("mismatch".to_owned()),
    ] {
        assert!(!e.code().is_empty());
        assert!(!e.to_string().is_empty());
    }

    let (json_err_code, _, json_err_str) = run_eval_cli(
        &[
            "--error-format",
            "json",
            "freeze",
            "--blind",
            blind_dir.to_str().ok_or("utf8")?,
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--control-wording-arm",
            CONTROL_WORDING_ARM,
            "--verified-checkpoints",
            vc_cas.to_str().ok_or("utf8")?,
            "--out",
            tmp.path().join("f_json_err.json").to_str().ok_or("utf8")?,
        ],
        &injected_sha(COMMIT_B),
    );
    assert_eq!(json_err_code, 1);
    assert!(
        json_err_str.contains("VERIFIED_CHECKPOINTS_MISMATCH"),
        "{json_err_str}"
    );

    let cas_v1_dir = tmp.path().join("cas_v1_single");
    fs::create_dir_all(&cas_v1_dir)?;
    let mut seal_v1_val: serde_json::Value = serde_json::from_slice(&seal_bytes)?;
    seal_v1_val["format_version"] = json!(1);
    let seal_v1_bytes = serde_json::to_vec(&seal_v1_val)?;
    let seal_v1_sha = sha256_hex(&seal_v1_bytes);
    fs::write(
        cas_v1_dir.join(format!("{save_sha}.save.json")),
        &save_bytes,
    )?;
    fs::write(
        cas_v1_dir.join(format!("{seal_v1_sha}.seal")),
        &seal_v1_bytes,
    )?;
    assert_eq!(
        run_verify_checkpoints(
            &VerifyCheckpointsOptions {
                checkpoints_dir: cas_v1_dir,
                out_path: tmp.path().join("vc_cas_v1.json"),
                blind_dir: None,
            },
            &src_a,
        )
        .err()
        .ok_or("single-pair cas v1 seal")?
        .code(),
        "INCOMPATIBLE_ENGINE_OR_SCHEMA_VERSION"
    );

    let file_parent = tmp.path().join("regular_file_parent");
    fs::write(&file_parent, b"x")?;
    let io_out_err = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: ckpts_cas_dir,
            out_path: file_parent.join("vc.json"),
            blind_dir: None,
        },
        &src_a,
    )
    .err()
    .ok_or("expected IO error creating parent dir under regular file")?;
    assert_eq!(io_out_err.code(), "EVAL_IO");
    Ok(())
}

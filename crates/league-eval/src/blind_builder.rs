//! Unblinded input builder for Decision Scorer S3a (`§3.1`, `§8.1`).
//!
//! Reads arm-labelled evaluation run directories (`play` checkpoints, `to-seat/` whispers,
//! `decisions/` replies, `branch` `decision.json`/`outcome.json`/`manifest.json`, and `relay/seats/`
//! tool-round sidecars), combines Sources 1–4 (`passive`, committed decisions, first drafts,
//! `ClubBot` + synthetics), deduplicates and OR-merges candidate flags, and writes disjoint:
//! - `<out-blind>/<state_id>.json` (`BlindScoringRequest` v1, zero arm/path/wording/tool metadata),
//! - `<out-arm-map>/<state_id>.json` (`ArmMapStateFile`, held outside `league-scorer`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use league_core::{EvalActSubmission, EvalHost, Seat};
use league_relay::{
    CallLogEntry, Round, SavedToolRoundCall, SavedToolRoundRecord, parse_preview_tool_arguments,
};
use league_scorer::{
    BLIND_SCORING_SCHEMA_VERSION, BlindCandidate, BlindScoringRequest, ScorerConfig,
    TALK_EXTRACTOR_VERSION_V2, candidate_id_for_submission, compute_pre_act_pair_sha256_hex,
    dedupe_candidates, generate_arm_independent_candidates, parse_blind_scoring_request_strict,
    sha256_hex, to_sorted_pretty_json, validate_state_id,
};
use serde::{Deserialize, Serialize};

use crate::error::EvalError;
use crate::records::{BranchArmOutcome, EvalArm, EvalDecisionRecord, EvalSetupFile};
use crate::reply::parse_eval_act_reply;
use crate::talk_extract::{
    extract_talk_commitments_for_state, resolve_eval_seat_guilds, talk_rules_v2_digest,
};

/// Options for [`run_build_blind_inputs`].
#[derive(Debug, Clone)]
pub struct BuildBlindInputsOptions {
    /// Input evaluation run directories (`--run <dir>...`, holding `play` and/or `branch` trees).
    pub run_dirs: Vec<PathBuf>,
    /// Output directory for anonymized `<state_id>.json` [`BlindScoringRequest`] files.
    pub out_blind_dir: PathBuf,
    /// Disjoint output directory for `<state_id>.json` [`ArmMapStateFile`] files.
    pub out_arm_map_dir: PathBuf,
    /// Optional content-addressed `<sha256>.{save.json,seal}` export directory for `league-scorer`.
    pub out_checkpoints_dir: Option<PathBuf>,
    /// Path to frozen [`ScorerConfig`] JSON.
    pub config_path: PathBuf,
    /// Explicit deterministic `scorer_seed`.
    pub scorer_seed: u64,
}

/// One `(arm, replicate)` provenance row stored inside `<out-arm-map>/<state_id>.json` (`§8.1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmMapEntry {
    /// Evaluated arm (`off`, `control`, `on`).
    pub arm: EvalArm,
    /// Zero-based deterministic replicate index within `(state_id, arm)`.
    pub replicate: u32,
    /// Canonical SHA-256 candidate ID of the committed reply (`EvalActSubmission::default()` on `no_reply`/refusal).
    pub committed_candidate_id: String,
    /// Canonical SHA-256 candidate ID of the first valid sent tool draft (`None` when absent).
    pub first_draft_candidate_id: Option<String>,
    /// Path to the unblinded `decision.json` record.
    pub decision_path: PathBuf,
    /// Path to the unblinded `outcome.json` record.
    pub outcome_path: PathBuf,
    /// Path to an arm-level or run-level `setup.json` (`EvalSetupFile`) when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_path: Option<PathBuf>,
    /// Path to `<arm>/manifest.json` (`EvalManifest`) when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_path: Option<PathBuf>,
}

/// Disjoint per-state mapping written under `<out-arm-map>/<state_id>.json` (`§8.1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmMapStateFile {
    /// Schema version (`BLIND_SCORING_SCHEMA_VERSION = 1`).
    pub schema_version: u32,
    /// Canonical arm-blind state identifier (`<source_game_id>-day-<DD>-seat-<idx>`).
    pub state_id: String,
    /// Source `game_id` (`R5`).
    pub source_game_id: String,
    /// 1-based pre-act checkpoint turn.
    pub day: u8,
    /// Zero-based focal seat index (`0..4`).
    pub focal_seat: u8,
    /// Recorded `scorer_seed` echoed in [`BlindScoringRequest`].
    pub scorer_seed: u64,
    /// SHA-256 hex digest of `day-DD-pre-act.save.json`.
    pub pre_act_save_sha256: String,
    /// SHA-256 hex digest of `day-DD-pre-act.seal`.
    pub pre_act_seal_sha256: String,
    /// Domain-separated checkpoint pair SHA-256 hex digest.
    pub pre_act_pair_sha256: String,
    /// SHA-256 hex digest of the exact bytes written to `blind_inputs/<state_id>.json`.
    pub blind_input_sha256: String,
    /// Sorted candidate IDs included in `blind_inputs/<state_id>.json`.
    pub blind_candidate_ids: Vec<String>,
    /// Ordered `(arm, replicate)` provenance entries for post-freeze reporting.
    pub entries: Vec<ArmMapEntry>,
}

/// Summary returned by [`run_build_blind_inputs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildBlindInputsReport {
    /// Number of states written to `--out-blind` and `--out-arm-map`.
    pub states_built: usize,
    /// Total `(arm, replicate)` entries mapped across all states.
    pub arm_entries_mapped: usize,
}

#[derive(Debug, Clone)]
struct DiscoveredOutcome {
    arm_dir: PathBuf,
    decision_path: PathBuf,
    outcome_path: PathBuf,
    outcome: BranchArmOutcome,
    decision: EvalDecisionRecord,
}

fn canonical_state_id(source_game_id: &str, day: u8, focal_seat: u8) -> Result<String, EvalError> {
    let sid = format!("{source_game_id}-day-{day:02}-seat-{focal_seat}");
    validate_state_id(&sid)?;
    Ok(sid)
}

fn is_wire_error_code(wire_json: Option<&str>, expected_code: &str) -> bool {
    let Some(raw) = wire_json else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| v.get("error_code")?.as_str().map(str::to_owned))
        .is_some_and(|c| c == expected_code)
}

fn scan_run_dir_recursive(
    dir: &Path,
    source_games: &mut BTreeMap<String, PathBuf>,
    outcomes: &mut Vec<DiscoveredOutcome>,
) -> Result<(), EvalError> {
    let setup_path = dir.join("setup.json");
    if setup_path.is_file() && dir.join("checkpoints").is_dir() {
        let raw = fs::read_to_string(&setup_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", setup_path.display())))?;
        if let Ok(setup) = serde_json::from_str::<EvalSetupFile>(&raw) {
            source_games
                .entry(setup.game_id)
                .or_insert_with(|| dir.to_path_buf());
        }
    }

    let decision_path = dir.join("decision.json");
    let outcome_path = dir.join("outcome.json");
    if decision_path.is_file() && outcome_path.is_file() {
        let out_raw = fs::read_to_string(&outcome_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", outcome_path.display())))?;
        let outcome: BranchArmOutcome = serde_json::from_str(&out_raw)
            .map_err(|e| EvalError::Io(format!("parse {}: {e}", outcome_path.display())))?;
        let dec_raw = fs::read_to_string(&decision_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", decision_path.display())))?;
        let decision: EvalDecisionRecord = serde_json::from_str(&dec_raw)
            .map_err(|e| EvalError::Io(format!("parse {}: {e}", decision_path.display())))?;
        outcomes.push(DiscoveredOutcome {
            arm_dir: dir.to_path_buf(),
            decision_path,
            outcome_path,
            outcome,
            decision,
        });
    }

    let mut subdirs = Vec::new();
    let entries =
        fs::read_dir(dir).map_err(|e| EvalError::Io(format!("read {}: {e}", dir.display())))?;
    for entry_res in entries {
        let entry = entry_res.map_err(|e| EvalError::Io(format!("read dir entry: {e}")))?;
        let ft = entry
            .file_type()
            .map_err(|e| EvalError::Io(format!("inspect entry type: {e}")))?;
        if ft.is_dir() {
            subdirs.push(entry.path());
        }
    }
    subdirs.sort();
    for sub in subdirs {
        scan_run_dir_recursive(&sub, source_games, outcomes)?;
    }
    Ok(())
}

fn committed_submission_from_decision(
    decision: &EvalDecisionRecord,
) -> Result<EvalActSubmission, EvalError> {
    if decision.no_reply {
        return Ok(EvalActSubmission::default());
    }
    let Some(raw) = decision.raw_reply.as_deref() else {
        return Ok(EvalActSubmission::default());
    };
    let parsed = parse_eval_act_reply(raw, decision.seat, decision.keeps_notes);
    if parsed.refusal_code.is_some() {
        return Ok(EvalActSubmission::default());
    }
    Ok(EvalActSubmission {
        market_actions: parsed.submission.market_actions,
        pass_b_actions: Vec::new(),
        bundle: parsed.submission.bundle,
    })
}

fn collect_matching_reservation_ids(
    calls_path: &Path,
    day_str: &str,
    guild: &str,
) -> Result<Option<BTreeSet<String>>, EvalError> {
    if !calls_path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(calls_path)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", calls_path.display())))?;
    let mut ids = BTreeSet::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let entry: CallLogEntry = serde_json::from_str(trimmed)
            .map_err(|e| EvalError::Io(format!("parse {} line: {e}", calls_path.display())))?;
        if entry.day == day_str
            && entry.round == Round::Act
            && entry.guild.eq_ignore_ascii_case(guild)
            && let Some(resv_id) = entry.reservation_id
        {
            ids.insert(resv_id);
        }
    }
    Ok(Some(ids))
}

fn extract_first_draft_submission(
    arm_dir: &Path,
    decision: &EvalDecisionRecord,
) -> Result<Option<EvalActSubmission>, EvalError> {
    let seats_dir = arm_dir.join("relay").join("seats");
    if !seats_dir.is_dir() {
        return Ok(None);
    }
    let day_str = format!("{:02}", decision.day);
    let prefix = format!("{}.day-{day_str}-act.tool-round-", decision.guild);
    let resv_filter = collect_matching_reservation_ids(
        &arm_dir.join("relay").join("calls.jsonl"),
        &day_str,
        &decision.guild,
    )?;

    let mut files = Vec::new();
    let entries = fs::read_dir(&seats_dir)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", seats_dir.display())))?;
    for entry_res in entries {
        let entry = entry_res.map_err(|e| EvalError::Io(format!("read seats entry: {e}")))?;
        let ft = entry
            .file_type()
            .map_err(|e| EvalError::Io(format!("inspect seats entry: {e}")))?;
        if !ft.is_file() {
            continue;
        }
        let fname = entry.file_name();
        let Some(name_str) = fname.to_str() else {
            continue;
        };
        if name_str.starts_with(&prefix) && name_str.ends_with(".json") {
            files.push(entry.path());
        }
    }
    files.sort();

    let mut flat_calls: Vec<(usize, u8, usize, SavedToolRoundCall)> = Vec::new();
    for path in files {
        let raw =
            fs::read(&path).map_err(|e| EvalError::Io(format!("read {}: {e}", path.display())))?;
        let rec: SavedToolRoundRecord = serde_json::from_slice(&raw)
            .map_err(|e| EvalError::Io(format!("parse {}: {e}", path.display())))?;
        if rec.day != day_str
            || rec.round != Round::Act
            || !rec.guild.eq_ignore_ascii_case(&decision.guild)
        {
            continue;
        }
        if let Some(ref allowed) = resv_filter
            && !allowed.contains(&rec.reservation_id)
        {
            continue;
        }
        for call in rec.calls {
            // Read the recorded `call_index` field rather than recomputing position.
            let idx = call.call_index;
            flat_calls.push((rec.round_index, rec.attempt, idx, call));
        }
    }

    flat_calls
        .sort_by_key(|(round_index, attempt, call_index, _)| (*round_index, *attempt, *call_index));

    for (_, _, _, call) in flat_calls {
        if !call.tool_result_sent {
            continue;
        }
        let wire = call.result_wire_json.as_deref();
        if is_wire_error_code(wire, "CALL_LIMIT_REACHED")
            || is_wire_error_code(wire, "MALFORMED_TOOL_CALL")
        {
            continue;
        }
        if let Some(parsed_req) = parse_preview_tool_arguments(&call.arguments, decision.seat) {
            return Ok(Some(EvalActSubmission {
                market_actions: parsed_req.market_actions,
                pass_b_actions: Vec::new(),
                bundle: parsed_req.bundle,
            }));
        }
    }

    Ok(None)
}

fn resolve_setup_path_for_arm(
    arm_dir: &Path,
    arm: EvalArm,
    source_game_dir: &Path,
) -> Option<PathBuf> {
    let direct = arm_dir.join("setup.json");
    if direct.is_file() {
        return Some(direct);
    }
    if let Some(parent) = arm_dir.parent() {
        let parent_setup = parent.join("setup.json");
        if parent_setup.is_file()
            && let Ok(raw) = fs::read_to_string(&parent_setup)
            && let Ok(parsed) = serde_json::from_str::<EvalSetupFile>(&raw)
            && parsed.arm == arm
        {
            return Some(parent_setup);
        }
    }
    let src_setup = source_game_dir.join("setup.json");
    if src_setup.is_file()
        && let Ok(raw) = fs::read_to_string(&src_setup)
        && let Ok(parsed) = serde_json::from_str::<EvalSetupFile>(&raw)
        && parsed.arm == arm
    {
        return Some(src_setup);
    }
    None
}

fn paths_are_disjoint(a: &Path, b: &Path) -> bool {
    let norm_a = a.to_string_lossy().replace('\\', "/");
    let norm_b = b.to_string_lossy().replace('\\', "/");
    let ta = norm_a.trim_end_matches('/');
    let tb = norm_b.trim_end_matches('/');
    ta != tb && !ta.starts_with(&format!("{tb}/")) && !tb.starts_with(&format!("{ta}/"))
}

/// Builds anonymized [`BlindScoringRequest`] files under `--out-blind` and disjoint
/// [`ArmMapStateFile`] provenance under `--out-arm-map` (`§3.1`, `§4.3`, `§8.1`).
pub fn run_build_blind_inputs(
    opts: &BuildBlindInputsOptions,
) -> Result<BuildBlindInputsReport, EvalError> {
    if opts.run_dirs.is_empty() {
        return Err(EvalError::Usage(
            "`build-blind-inputs` requires at least one `--run <dir>`".to_owned(),
        ));
    }
    if !paths_are_disjoint(&opts.out_blind_dir, &opts.out_arm_map_dir) {
        return Err(EvalError::Usage(
            "`--out-blind` and `--out-arm-map` must be separate disjoint directories (§8.1)"
                .to_owned(),
        ));
    }

    let cfg_bytes = fs::read(&opts.config_path)
        .map_err(|e| EvalError::Io(format!("read {}: {e}", opts.config_path.display())))?;
    let config: ScorerConfig = serde_json::from_slice(&cfg_bytes)
        .map_err(|e| EvalError::Io(format!("parse ScorerConfig JSON: {e}")))?;
    config.validate()?;
    if config.talk_extractor_version != TALK_EXTRACTOR_VERSION_V2 {
        return Err(EvalError::SchemaValidation(format!(
            "unsupported talk_extractor_version `{}` (expected `{TALK_EXTRACTOR_VERSION_V2}`)",
            config.talk_extractor_version
        )));
    }
    let expected_talk_digest = talk_rules_v2_digest()?;
    if config.talk_extractor_rules_digest != expected_talk_digest {
        return Err(EvalError::SchemaValidation(format!(
            "config talk_extractor_rules_digest `{}` != `{expected_talk_digest}`",
            config.talk_extractor_rules_digest
        )));
    }
    let config_sha = config.sha256_hex()?;

    let mut run_dirs_sorted = opts.run_dirs.clone();
    run_dirs_sorted.sort();
    let mut source_games = BTreeMap::new();
    let mut discovered = Vec::new();
    for dir in &run_dirs_sorted {
        if !dir.is_dir() {
            return Err(EvalError::MissingSourceSetup(format!(
                "--run directory does not exist: {}",
                dir.display()
            )));
        }
        scan_run_dir_recursive(dir, &mut source_games, &mut discovered)?;
    }
    if discovered.is_empty() {
        return Err(EvalError::MissingSourceSetup(
            "no `(decision.json, outcome.json)` arm outcomes found in `--run` directories"
                .to_owned(),
        ));
    }

    let mut by_state: BTreeMap<String, Vec<DiscoveredOutcome>> = BTreeMap::new();
    for item in discovered {
        let sid = canonical_state_id(
            &item.outcome.source_game_id,
            item.outcome.day,
            item.outcome.focal_seat,
        )?;
        by_state.entry(sid).or_default().push(item);
    }

    fs::create_dir_all(&opts.out_blind_dir).map_err(|e| {
        EvalError::Io(format!(
            "create --out-blind {}: {e}",
            opts.out_blind_dir.display()
        ))
    })?;
    fs::create_dir_all(&opts.out_arm_map_dir).map_err(|e| {
        EvalError::Io(format!(
            "create --out-arm-map {}: {e}",
            opts.out_arm_map_dir.display()
        ))
    })?;
    if let Some(ref ckpt_out) = opts.out_checkpoints_dir {
        fs::create_dir_all(ckpt_out).map_err(|e| {
            EvalError::Io(format!(
                "create --out-checkpoints {}: {e}",
                ckpt_out.display()
            ))
        })?;
    }

    let mut seen_ci_ids = BTreeSet::new();
    let mut total_entries = 0_usize;

    for (state_id, mut items) in by_state {
        if !seen_ci_ids.insert(state_id.to_ascii_lowercase()) {
            return Err(EvalError::SchemaValidation(format!(
                "case-insensitive state_id collision for `{state_id}`"
            )));
        }
        items.sort_by(|a, b| {
            (a.outcome.arm.as_str(), &a.decision_path, &a.outcome_path).cmp(&(
                b.outcome.arm.as_str(),
                &b.decision_path,
                &b.outcome_path,
            ))
        });

        let source_game_id = items[0].outcome.source_game_id.clone();
        let day = items[0].outcome.day;
        let focal_seat = items[0].outcome.focal_seat;
        let Some(source_game_dir) = source_games.get(&source_game_id) else {
            return Err(EvalError::MissingSourceSetup(format!(
                "missing source game directory with setup.json + checkpoints/ for game_id `{source_game_id}`"
            )));
        };

        let day_str = format!("{day:02}");
        let save_path = source_game_dir
            .join("checkpoints")
            .join(format!("day-{day_str}-pre-act.save.json"));
        let seal_path = source_game_dir
            .join("checkpoints")
            .join(format!("day-{day_str}-pre-act.seal"));
        if !save_path.is_file() || !seal_path.is_file() {
            return Err(EvalError::CheckpointNotFound(format!(
                "missing pre-act checkpoint {} / {}",
                save_path.display(),
                seal_path.display()
            )));
        }

        let save_bytes = fs::read(&save_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", save_path.display())))?;
        let seal_bytes = fs::read(&seal_path)
            .map_err(|e| EvalError::Io(format!("read {}: {e}", seal_path.display())))?;
        let save_sha = sha256_hex(&save_bytes);
        let seal_sha = sha256_hex(&seal_bytes);
        let pair_sha = compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes);

        if let Some(ref ckpt_out) = opts.out_checkpoints_dir {
            fs::write(ckpt_out.join(format!("{save_sha}.save.json")), &save_bytes).map_err(
                |e| EvalError::Io(format!("write content-addressed checkpoint save: {e}")),
            )?;
            fs::write(ckpt_out.join(format!("{seal_sha}.seal")), &seal_bytes).map_err(|e| {
                EvalError::Io(format!("write content-addressed checkpoint seal: {e}"))
            })?;
        }

        let host = EvalHost::fork_from_checkpoint(&save_path, &seal_path)?;
        let obs = host.observe_seat(Seat(focal_seat))?;
        let own_mandate = *host.mandate_for_seat(Seat(focal_seat))?;
        let bot_tie_seed = host.bot_tie_seed();

        let seat_guilds = resolve_eval_seat_guilds(source_game_dir, obs.spec.seat_count)?;
        let talk_commitments = extract_talk_commitments_for_state(
            source_game_dir,
            day,
            focal_seat,
            &seat_guilds,
            &obs,
        )?;

        let mut candidates =
            generate_arm_independent_candidates(&obs, &own_mandate, &config, bot_tie_seed)?;
        let mut arm_map_entries = Vec::with_capacity(items.len());
        let mut rep_by_arm: BTreeMap<EvalArm, u32> = BTreeMap::new();

        for item in items {
            let committed_sub = committed_submission_from_decision(&item.decision)?;
            let committed_id = candidate_id_for_submission(&committed_sub)?;
            candidates.push(BlindCandidate {
                candidate_id: committed_id.clone(),
                submission: committed_sub,
                in_span: true,
                is_arm_independent: false,
            });

            let draft_sub_opt = extract_first_draft_submission(&item.arm_dir, &item.decision)?;
            let first_draft_id = if let Some(draft_sub) = draft_sub_opt {
                let draft_id = candidate_id_for_submission(&draft_sub)?;
                candidates.push(BlindCandidate {
                    candidate_id: draft_id.clone(),
                    submission: draft_sub,
                    in_span: false,
                    is_arm_independent: false,
                });
                Some(draft_id)
            } else {
                None
            };

            let rep_slot = rep_by_arm.entry(item.outcome.arm).or_insert(0);
            let replicate = *rep_slot;
            *rep_slot = rep_slot.saturating_add(1);

            let manifest_candidate = item.arm_dir.join("manifest.json");
            let manifest_path = manifest_candidate.is_file().then_some(manifest_candidate);
            let setup_path =
                resolve_setup_path_for_arm(&item.arm_dir, item.outcome.arm, source_game_dir);

            arm_map_entries.push(ArmMapEntry {
                arm: item.outcome.arm,
                replicate,
                committed_candidate_id: committed_id,
                first_draft_candidate_id: first_draft_id,
                decision_path: item.decision_path,
                outcome_path: item.outcome_path,
                setup_path,
                manifest_path,
            });
        }

        let deduped = dedupe_candidates(candidates)?;
        let req = BlindScoringRequest {
            schema_version: BLIND_SCORING_SCHEMA_VERSION,
            state_id: state_id.clone(),
            pre_act_save_sha256: save_sha.clone(),
            pre_act_seal_sha256: seal_sha.clone(),
            pre_act_pair_sha256: pair_sha.clone(),
            focal_seat,
            talk_commitments,
            candidates: deduped,
            scorer_seed: opts.scorer_seed,
            scorer_config_sha256: config_sha.clone(),
        };
        let blind_json = to_sorted_pretty_json(&req)?;
        let parsed_strict = parse_blind_scoring_request_strict(blind_json.as_bytes())?;
        parsed_strict.validate(&config)?;

        let blind_path = opts.out_blind_dir.join(format!("{state_id}.json"));
        fs::write(&blind_path, blind_json.as_bytes())
            .map_err(|e| EvalError::Io(format!("write {}: {e}", blind_path.display())))?;
        let blind_input_sha256 = sha256_hex(blind_json.as_bytes());
        let blind_candidate_ids: Vec<String> = parsed_strict
            .candidates
            .iter()
            .map(|c| c.candidate_id.clone())
            .collect();

        let arm_map_state = ArmMapStateFile {
            schema_version: BLIND_SCORING_SCHEMA_VERSION,
            state_id: state_id.clone(),
            source_game_id,
            day,
            focal_seat,
            scorer_seed: opts.scorer_seed,
            pre_act_save_sha256: save_sha,
            pre_act_seal_sha256: seal_sha,
            pre_act_pair_sha256: pair_sha,
            blind_input_sha256,
            blind_candidate_ids,
            entries: arm_map_entries,
        };
        let arm_map_json = to_sorted_pretty_json(&arm_map_state)?;
        let arm_map_path = opts.out_arm_map_dir.join(format!("{state_id}.json"));
        fs::write(&arm_map_path, arm_map_json.as_bytes())
            .map_err(|e| EvalError::Io(format!("write {}: {e}", arm_map_path.display())))?;
        total_entries += arm_map_state.entries.len();
    }

    Ok(BuildBlindInputsReport {
        states_built: seen_ci_ids.len(),
        arm_entries_mapped: total_entries,
    })
}

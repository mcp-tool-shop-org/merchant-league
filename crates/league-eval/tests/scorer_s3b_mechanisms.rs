//! Tests for §7.2 unblinded mechanism measures, doc parity, missing-data reporting, and
//! in-memory prose-isolation guarantees.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use league_core::{
    AssetId, EvalActSubmission, EvalHost, EvalSeatApplyReport, MarketAction, OfferProposal,
    OfferSide, PublicSweepReceipt, SealedOrderBundle, Seat, TableId,
};
use league_eval::{
    ArmMapEntry, ArmMapStateFile, BranchArmOutcome, CRATE_GIT_SHA, CommunicationMode,
    CrateGitShaSource, DESCRIPTIVE_ONLY_INFERENTIAL_STATUS, EvalArm, EvalDecisionRecord,
    EvalManifest, EvalManifestSeat, FreezeOptions, HTTP_REPLY_OK_STATUS, JoinOptions,
    JoinedDecisionsFile, MECHANISM_DATA_LIMITS, PASS_B_LIMIT_STATEMENT, PerArmSummaryFile,
    REFUSED_BUNDLE_LEGS_MIN_COUNT, RefusalCodeCounts, RefusedBundleRecord,
    TALK_COMMITMENT_HONOUR_WINDOW_DAYS, TALK_COMMITMENT_MIN_FILL_BPS,
    TALK_COMMITMENT_PRICE_TOLERANCE_MILLI_CRED, TOOL_CALL_USED_MIN_COUNT, VerifyCheckpointsOptions,
    WrongPartCounter, execute_cli_with_git_sha, measure_reasoning_length_in_memory, run_freeze,
    run_join, run_verify_checkpoints, summarize_arm_mechanisms,
};
use league_relay::{
    CONTROL_WORDING_ARM, CallLogEntry, NegotiationInputs, Round, SavedHttpReplyRecord,
    SavedToolRoundRecord, UntrustedWhisper, execute_preview_tool_call,
};
use league_scorer::{
    BLIND_SCORING_SCHEMA_VERSION, BlindCandidate, BlindScoringRequest, CandidateScoreOutput,
    PrescreenSummary, ScorerConfig, StateScoreOutput, TalkExtractionCounts, WinnersCurseSummary,
    candidate_id_for_submission, compute_pre_act_pair_sha256_hex, sha256_hex,
    to_sorted_pretty_json,
};
use league_seat_types::{
    BrokenPromiseRecord, FillKind, FillRecord, FinalScoreboard, SeatScoreEntry, UNIT_SCALE,
};
use serde::de::DeserializeOwned;
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const FALLBACK_GIT_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct TmpDir(PathBuf);

impl TmpDir {
    fn new(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let seq = TMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("ml_s3b_{label}_{}_{}", std::process::id(), seq));
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

fn clean_build_sha() -> &'static str {
    if CRATE_GIT_SHA.len() == 40 && CRATE_GIT_SHA.bytes().all(|b| b.is_ascii_hexdigit()) {
        CRATE_GIT_SHA
    } else {
        FALLBACK_GIT_SHA
    }
}

fn verify_source_ckpts(
    src_dir: &Path,
    out_path: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let _ = run_verify_checkpoints(
        &VerifyCheckpointsOptions {
            checkpoints_dir: src_dir.join("checkpoints"),
            out_path: out_path.to_path_buf(),
            blind_dir: None,
        },
        &clean_git_sha_source(),
    )?;
    Ok(out_path.to_path_buf())
}

fn clean_git_sha_source() -> CrateGitShaSource<'static> {
    let built_sha =
        if CRATE_GIT_SHA.len() == 40 && CRATE_GIT_SHA.bytes().all(|b| b.is_ascii_hexdigit()) {
            CRATE_GIT_SHA
        } else {
            FALLBACK_GIT_SHA
        };
    CrateGitShaSource::Injected {
        built_sha,
        live_head: Some(built_sha),
        is_dirty: Some(false),
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

fn sample_submission(tag: i64) -> EvalActSubmission {
    if tag == 0 {
        return EvalActSubmission::default();
    }
    EvalActSubmission {
        market_actions: vec![MarketAction::PostOffer {
            seat: 0,
            order: OfferProposal {
                side: OfferSide::Sell,
                good: AssetId::Raw(0),
                amount: tag * UNIT_SCALE,
                price_per_unit_cred: 10 * UNIT_SCALE,
                domain_id: None,
                is_hybrid: false,
            },
        }],
        pass_b_actions: Vec::new(),
        bundle: SealedOrderBundle::default(),
    }
}

fn default_receipt(turn: u8) -> PublicSweepReceipt {
    PublicSweepReceipt {
        turn,
        non_reveal_seats: Vec::new(),
        regent_acted_seats: Vec::new(),
        converted_to_ai: Vec::new(),
        escrows_finished: Vec::new(),
        escrows_cancelled_or_forfeited: Vec::new(),
        atomic_swaps_settled: 0,
        converter_batches_run: 0,
        converter_orders_clamped: 0,
        checks_cashed: Vec::new(),
        checks_bounced: Vec::new(),
        cartel_evictions: Vec::new(),
        projects_completed: 0,
        project_orders_clamped: 0,
        total_upkeep_shortfall_debt: 0,
        total_spoilage_clawed_back: 0,
        broken_promises: Vec::new(),
        elected_officers: None,
        charters_awarded: Vec::new(),
        public_forecasts: Vec::new(),
        newly_activated_shocks: Vec::new(),
        cred_debt_repaid: 0,
        charter_cred_burned: 0,
        town_import_cred_spent: 0,
        works_stipend_cred_minted: 0,
        batch_trades_cleared: 0,
        fills: Vec::new(),
        wash_freeze_settlements: Vec::new(),
        season_closed: false,
        pbrs_deltas: vec![0; 4],
    }
}

fn base_decision_record(arm: EvalArm, day: u8, seat: u8, guild: &str) -> EvalDecisionRecord {
    EvalDecisionRecord {
        game_id: "g_s3b".to_owned(),
        table_id: 9200,
        day,
        round: Round::Act,
        seat,
        guild: guild.to_owned(),
        communication: CommunicationMode::Talk,
        arm,
        seat_kind: "Model".to_owned(),
        keeps_notes: true,
        notes_present: false,
        raw_reply: None,
        discarded_reply_path: None,
        discarded_reply_sha256: None,
        attempts: 1,
        failed_attempts: Vec::new(),
        prompt_tokens: 10,
        completion_tokens: 0,
        wall_secs: Some(0.1),
        tool_calls_used: 0,
        parse_failure: false,
        refused_market_actions: 0,
        refused_bundle_legs: 0,
        cleared_non_empty_actions: false,
        intermediate_bundle_sha256s: Vec::new(),
        prompt_token_ids_sha256s: Vec::new(),
        prompt_token_ids_source: None,
        prompt_token_ids_template_check: None,
        reply_token_ids: Vec::new(),
        reply_top_logprobs: Vec::new(),
        loaded_history_sha256: BTreeMap::new(),
        refusal_code: None,
        refusal_reason: None,
        invalid_recipients: Vec::new(),
        no_reply: false,
        apply_report: None,
        refused_bundle: None,
        club_bot_baseline: None,
    }
}

fn verify_sec72_constants_against_doc(doc_text: &str) -> Result<(), String> {
    let (_, after_heading) = doc_text
        .split_once("#### 7.2.1 Named Constants")
        .ok_or_else(|| "missing `#### 7.2.1 Named Constants` heading".to_owned())?;
    let (section, _) = after_heading
        .split_once("#### 7.2.2")
        .ok_or_else(|| "missing `#### 7.2.2` delimiter".to_owned())?;

    let mut parsed: BTreeMap<String, (String, String)> = BTreeMap::new();
    for line in section.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("| `") {
            continue;
        }
        let cols: Vec<&str> = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if cols.len() < 4 {
            continue;
        }
        let name = cols[0].trim_matches('`').to_owned();
        let ty = cols[1].trim_matches('`').to_owned();
        let val = cols[2].trim_matches('`').to_owned();
        parsed.insert(name, (ty, val));
    }

    let expected = [
        (
            "TOOL_CALL_USED_MIN_COUNT",
            "u32",
            TOOL_CALL_USED_MIN_COUNT.to_string(),
        ),
        (
            "HTTP_REPLY_OK_STATUS",
            "u16",
            HTTP_REPLY_OK_STATUS.to_string(),
        ),
        (
            "REFUSED_BUNDLE_LEGS_MIN_COUNT",
            "u32",
            REFUSED_BUNDLE_LEGS_MIN_COUNT.to_string(),
        ),
        (
            "TALK_COMMITMENT_HONOUR_WINDOW_DAYS",
            "u8",
            TALK_COMMITMENT_HONOUR_WINDOW_DAYS.to_string(),
        ),
        ("TALK_COMMITMENT_MIN_FILL_BPS", "u32", "10_000".to_owned()),
        (
            "TALK_COMMITMENT_PRICE_TOLERANCE_MILLI_CRED",
            "i64",
            TALK_COMMITMENT_PRICE_TOLERANCE_MILLI_CRED.to_string(),
        ),
    ];
    if parsed.len() != expected.len() {
        return Err(format!(
            "§7.2.1 table count {} != expected {}",
            parsed.len(),
            expected.len()
        ));
    }
    if TALK_COMMITMENT_MIN_FILL_BPS != 10_000 {
        return Err("TALK_COMMITMENT_MIN_FILL_BPS != 10_000".to_owned());
    }
    for (name, ty, val) in expected {
        let Some((doc_ty, doc_val)) = parsed.get(name) else {
            return Err(format!("missing `{name}` in §7.2.1 table"));
        };
        if doc_ty != ty || *doc_val != val {
            return Err(format!(
                "`{name}` doc=({doc_ty}, {doc_val}) != code=({ty}, {val})"
            ));
        }
    }
    Ok(())
}

fn parse_sec72_data_limits_from_doc(doc_text: &str) -> Result<Vec<String>, String> {
    let (_, after_item) = doc_text
        .split_once("5. **Plain statement of what the recorded data cannot tell (never guessed):**")
        .ok_or_else(|| "missing §7.2.3 item 5 heading".to_owned())?;
    let (section, _) = after_item
        .split_once("#### 7.2.4")
        .ok_or_else(|| "missing `#### 7.2.4` delimiter".to_owned())?;
    let mut items = Vec::new();
    for line in section.lines() {
        if let Some(rest) = line.strip_prefix("   - ") {
            items.push(rest.trim().to_owned());
        }
    }
    Ok(items)
}

#[test]
fn doc_sec72_constants_match_code_and_reject_drift() -> TestResult {
    let doc_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/decision-scorer.md");
    let doc_text = fs::read_to_string(&doc_path)?;
    verify_sec72_constants_against_doc(&doc_text).map_err(std::io::Error::other)?;

    let drifted = doc_text.replacen("| `10_000` |", "| `5_000` |", 1);
    assert_ne!(drifted, doc_text);
    assert!(verify_sec72_constants_against_doc(&drifted).is_err());
    Ok(())
}

#[test]
fn doc_sec72_data_limits_match_constant_and_reject_drift() -> TestResult {
    let doc_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/decision-scorer.md");
    let doc_text = fs::read_to_string(&doc_path)?;
    let doc_items = parse_sec72_data_limits_from_doc(&doc_text).map_err(std::io::Error::other)?;
    let code_items: Vec<String> = MECHANISM_DATA_LIMITS
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert_eq!(doc_items, code_items);
    assert_eq!(doc_items.len(), 7);

    let drifted = doc_text.replacen(
        "**Multi-day horizon beyond `day`:**",
        "**Drifted bullet:**",
        1,
    );
    let drifted_items =
        parse_sec72_data_limits_from_doc(&drifted).map_err(std::io::Error::other)?;
    assert_ne!(drifted_items, code_items);
    Ok(())
}

fn write_minimal_source_game(
    dir: &Path,
    host: &mut EvalHost,
    arm: EvalArm,
    game_id: &str,
) -> Result<(String, String, String), Box<dyn std::error::Error>> {
    fs::create_dir_all(dir.join("checkpoints"))?;
    fs::create_dir_all(dir.join("decisions"))?;
    fs::create_dir_all(dir.join("to-seat"))?;

    host.set_engine_commit(clean_build_sha())?;
    let (save_path, seal_path) = host.write_pre_act_checkpoint(&dir.join("checkpoints"))?;
    let save_bytes = fs::read(&save_path)?;
    let seal_bytes = fs::read(&seal_path)?;
    let save_sha = sha256_hex(&save_bytes);
    let seal_sha = sha256_hex(&seal_bytes);
    let pair_sha = compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes);

    let setup = json!({
        "game_id": game_id,
        "table_id": 9200_u64,
        "seed": 11_u64,
        "turns": 1_u8,
        "seat_count": 4_u8,
        "arm": arm,
        "eval_reply_wording_arm": "v2_eval_reply_2026_11_21_r1",
        "relay_wording_arm": CONTROL_WORDING_ARM,
        "wrapper_version": "r3",
        "wrapper_sha256": sha256_hex(b"wrap"),
        "memory_mode": "snapshot",
        "seats_sha256": sha256_hex(b"seats"),
    });
    fs::write(dir.join("setup.json"), to_sorted_pretty_json(&setup)?)?;

    let manifest = EvalManifest {
        game_id: game_id.to_owned(),
        table_id: 9200,
        seed: 11,
        communication: CommunicationMode::Talk,
        arm,
        eval_reply_wording_arm: "v2_eval_reply_2026_11_21_r1".to_owned(),
        relay_wording_arm: CONTROL_WORDING_ARM.to_owned(),
        wrapper_version: "r3".to_owned(),
        wrapper_sha256: sha256_hex(b"wrap"),
        pass_b_limit: PASS_B_LIMIT_STATEMENT.to_owned(),
        played_turns: 1,
        crate_git_sha: FALLBACK_GIT_SHA.to_owned(),
        wording_digests: None,
        serving: None,
        serving_env_snapshot: BTreeMap::new(),
        seats: vec![
            EvalManifestSeat {
                seat: 0,
                guild: "Foundry".to_owned(),
                backend: "local".to_owned(),
                model: "m".to_owned(),
                keeps_notes: true,
                has_regent: false,
            },
            EvalManifestSeat {
                seat: 1,
                guild: "Quarry".to_owned(),
                backend: "file".to_owned(),
                model: "ClubBot".to_owned(),
                keeps_notes: false,
                has_regent: true,
            },
            EvalManifestSeat {
                seat: 2,
                guild: "Hearth".to_owned(),
                backend: "file".to_owned(),
                model: "ClubBot".to_owned(),
                keeps_notes: false,
                has_regent: true,
            },
            EvalManifestSeat {
                seat: 3,
                guild: "Wharf".to_owned(),
                backend: "file".to_owned(),
                model: "ClubBot".to_owned(),
                keeps_notes: false,
                has_regent: true,
            },
        ],
        seat_regents: BTreeMap::new(),
        turn_regent_audit: Vec::new(),
        voided: false,
        void_error_code: None,
        voided_games_by_arm: BTreeMap::new(),
        refusal_counts_by_seat: BTreeMap::new(),
        refusal_counts_by_arm: BTreeMap::new(),
        wrong_part_refusals: WrongPartCounter::default(),
    };
    let _: RefusalCodeCounts = RefusalCodeCounts::default();
    fs::write(dir.join("manifest.json"), to_sorted_pretty_json(&manifest)?)?;

    Ok((save_sha, seal_sha, pair_sha))
}

fn write_arm_outcome_pair(
    arm_dir: &Path,
    decision: &EvalDecisionRecord,
    outcome: &BranchArmOutcome,
) -> Result<(PathBuf, PathBuf), Box<dyn std::error::Error>> {
    fs::create_dir_all(arm_dir)?;
    let dec_path = arm_dir.join("decision.json");
    let out_path = arm_dir.join("outcome.json");
    fs::write(&dec_path, to_sorted_pretty_json(decision)?)?;
    fs::write(&out_path, to_sorted_pretty_json(outcome)?)?;
    Ok((dec_path, out_path))
}

#[allow(clippy::too_many_arguments)]
fn build_blind_and_score_files(
    blind_dir: &Path,
    arm_map_dir: &Path,
    scores_dir: &Path,
    state_id: &str,
    game_id: &str,
    day: u8,
    focal_seat: u8,
    save_sha: &str,
    seal_sha: &str,
    pair_sha: &str,
    cfg: &ScorerConfig,
    entries: Vec<ArmMapEntry>,
    cand_ev: Vec<(EvalActSubmission, f64, Option<f64>)>,
    degenerate_flat_set: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(blind_dir)?;
    fs::create_dir_all(arm_map_dir)?;
    fs::create_dir_all(scores_dir)?;

    let mut blind_candidates = Vec::new();
    let mut score_candidates = Vec::new();
    for (sub, ev, sn) in &cand_ev {
        let cid = candidate_id_for_submission(sub)?;
        blind_candidates.push(BlindCandidate {
            candidate_id: cid.clone(),
            in_span: true,
            is_arm_independent: false,
            submission: sub.clone(),
        });
        score_candidates.push(CandidateScoreOutput {
            candidate_id: cid,
            ev_total_equity: *ev,
            in_span: true,
            is_arm_independent: false,
            mean_mandate_bonus: 0.0,
            mean_net_asset_equity: *ev,
            mean_project_equity: 0.0,
            paired_se_equity: 0.0,
            regret_equity: 0.0,
            score_norm: *sn,
            sensitivity_indep_percentile: Some(0.5),
            sensitivity_indep_rank: Some(1),
        });
    }
    blind_candidates.sort_by(|a, b| a.candidate_id.cmp(&b.candidate_id));
    score_candidates.sort_by(|a, b| a.candidate_id.cmp(&b.candidate_id));
    let blind_cids: Vec<String> = blind_candidates
        .iter()
        .map(|c| c.candidate_id.clone())
        .collect();
    let best_cid = blind_cids[0].clone();

    let req = BlindScoringRequest {
        candidates: blind_candidates,
        focal_seat,
        pre_act_pair_sha256: pair_sha.to_owned(),
        pre_act_save_sha256: save_sha.to_owned(),
        pre_act_seal_sha256: seal_sha.to_owned(),
        schema_version: BLIND_SCORING_SCHEMA_VERSION,
        scorer_config_sha256: cfg.sha256_hex()?,
        scorer_seed: 7,
        state_id: state_id.to_owned(),
        talk_commitments: Vec::new(),
        talk_extraction_counts: TalkExtractionCounts::default(),
    };
    let req_bytes = to_sorted_pretty_json(&req)?;
    let req_sha = sha256_hex(req_bytes.as_bytes());
    fs::write(blind_dir.join(format!("{state_id}.json")), req_bytes)?;

    let arm_map = ArmMapStateFile {
        schema_version: BLIND_SCORING_SCHEMA_VERSION,
        state_id: state_id.to_owned(),
        communication: CommunicationMode::Talk,
        source_game_id: game_id.to_owned(),
        day,
        focal_seat,
        scorer_seed: 7,
        pre_act_save_sha256: save_sha.to_owned(),
        pre_act_seal_sha256: seal_sha.to_owned(),
        pre_act_pair_sha256: pair_sha.to_owned(),
        blind_input_sha256: req_sha.clone(),
        blind_candidate_ids: blind_cids,
        entries,
    };
    fs::write(
        arm_map_dir.join(format!("{state_id}.json")),
        to_sorted_pretty_json(&arm_map)?,
    )?;

    let score = StateScoreOutput {
        blind_request_sha256: req_sha,
        c_best: best_cid.clone(),
        candidates: score_candidates,
        day,
        degenerate_flat_set,
        focal_seat,
        pre_act_pair_sha256: pair_sha.to_owned(),
        pre_act_save_sha256: save_sha.to_owned(),
        pre_act_seal_sha256: seal_sha.to_owned(),
        prescreen: PrescreenSummary {
            excluded_by_prescreen: degenerate_flat_set,
            is_flat_indep: degenerate_flat_set,
            n_feasible_indep: 1,
            span_indep: if degenerate_flat_set { 0.0 } else { 10.0 },
            v_max_indep: Some(15.0),
            v_min_indep: Some(5.0),
        },
        scenarios_m: 2,
        schema_version: BLIND_SCORING_SCHEMA_VERSION,
        scorer_config_sha256: cfg.sha256_hex()?,
        scorer_seed: 7,
        state_id: state_id.to_owned(),
        talk_extraction_counts: TalkExtractionCounts::default(),
        v_max: 20.0,
        v_min: 0.0,
        winners_curse: WinnersCurseSummary {
            bias_estimate_equity: 0.0,
            c_star_half_a: best_cid.clone(),
            c_star_half_b: best_cid,
            diff_a_to_b_equity: 0.0,
            diff_b_to_a_equity: 0.0,
            half_scenarios: 1,
            is_half_m_upper_estimate: true,
        },
    };
    fs::write(
        scores_dir.join(format!("{state_id}.json")),
        to_sorted_pretty_json(&score)?,
    )?;
    Ok(())
}

#[test]
fn measures_1_to_4_and_missing_data_in_join_report() -> TestResult {
    let tmp = TmpDir::new("m1_m4")?;
    let mut host = EvalHost::from_standard_setup(TableId(9200), 4, 1, 11)?;
    let src_dir = tmp.path().join("src_game");
    let (save_sha, seal_sha, pair_sha) =
        write_minimal_source_game(&src_dir, &mut host, EvalArm::Control, "g_m14")?;

    let s0 = sample_submission(0);
    let s1 = sample_submission(1);
    let s2 = sample_submission(2);
    let c0 = candidate_id_for_submission(&s0)?;
    let c1 = candidate_id_for_submission(&s1)?;
    let c2 = candidate_id_for_submission(&s2)?;

    // Off arm: no tools, no_reply, refusal + refused market actions + whole-bundle rejection, no relay/seats.
    let mut dec_off = base_decision_record(EvalArm::Off, 1, 0, "Foundry");
    dec_off.no_reply = true;
    dec_off.parse_failure = true;
    dec_off.refusal_code = Some("ACT_REPLY_INVALID".to_owned());
    dec_off.refused_market_actions = 2;
    dec_off.completion_tokens = 14;
    dec_off.refused_bundle = Some(RefusedBundleRecord {
        submitted_bundle: SealedOrderBundle::default(),
        error_code: "BAD_BUNDLE".to_owned(),
        bad_leg_index: "0".to_owned(),
    });
    dec_off.refused_bundle_legs = 2;
    let out_off = BranchArmOutcome {
        source_game_id: "g_m14".to_owned(),
        table_id: 9200,
        day: 1,
        focal_seat: 0,
        focal_guild: "Foundry".to_owned(),
        communication: CommunicationMode::Talk,
        arm: EvalArm::Off,
        decision: dec_off.clone(),
        receipt: default_receipt(1),
    };
    let (dec_off_p, out_off_p) =
        write_arm_outcome_pair(&tmp.path().join("arm_off"), &dec_off, &out_off)?;

    // Control arm State 1: tool-round reasoning (4 chars) deduplicated with matching reservation HTTP reply + fallback reasoning_content (6 chars).
    let arm_ctrl_1 = tmp.path().join("arm_ctrl_1");
    let mut dec_ctrl_1 = base_decision_record(EvalArm::Control, 1, 0, "Foundry");
    dec_ctrl_1.tool_calls_used = 2;
    dec_ctrl_1.completion_tokens = 30;
    dec_ctrl_1.reply_token_ids = vec![vec![1, 2, 3], vec![4, 5]];
    let out_ctrl_1 = BranchArmOutcome {
        source_game_id: "g_m14".to_owned(),
        table_id: 9200,
        day: 1,
        focal_seat: 0,
        focal_guild: "Foundry".to_owned(),
        communication: CommunicationMode::Talk,
        arm: EvalArm::Control,
        decision: dec_ctrl_1.clone(),
        receipt: default_receipt(1),
    };
    let (dec_c1_p, out_c1_p) = write_arm_outcome_pair(&arm_ctrl_1, &dec_ctrl_1, &out_ctrl_1)?;
    fs::create_dir_all(arm_ctrl_1.join("relay/seats"))?;
    let calls_lines = [
        serde_json::to_string(&CallLogEntry {
            time_utc: "2026-11-23T00:00:00Z".to_owned(),
            day: "01".to_owned(),
            round: Round::Act,
            guild: "Foundry".to_owned(),
            attempt: 1,
            trigger: "turn".to_owned(),
            model_requested: "m".to_owned(),
            model_returned: Some("m".to_owned()),
            request_body_sha256: "00".repeat(32),
            http_reply_sha256: None,
            reply_sha256: None,
            prompt_tokens: Some(10),
            completion_tokens: Some(10),
            total_tokens: Some(20),
            reasoning_tokens: None,
            cost_unit: league_relay::CostUnit::default(),
            wall_secs: Some(0.1),
            cost: None,
            charged_credits: 0.0,
            latency_ms: 100,
            outcome: "TOOL_CALL".to_owned(),
            error: None,
            reservation_id: Some("resv-t0".to_owned()),
        })?,
        serde_json::to_string(&CallLogEntry {
            time_utc: "2026-11-23T00:00:01Z".to_owned(),
            day: "01".to_owned(),
            round: Round::Act,
            guild: "Foundry".to_owned(),
            attempt: 1,
            trigger: "turn".to_owned(),
            model_requested: "m".to_owned(),
            model_returned: Some("m".to_owned()),
            request_body_sha256: "00".repeat(32),
            http_reply_sha256: None,
            reply_sha256: None,
            prompt_tokens: Some(10),
            completion_tokens: Some(20),
            total_tokens: Some(30),
            reasoning_tokens: None,
            cost_unit: league_relay::CostUnit::default(),
            wall_secs: Some(0.1),
            cost: None,
            charged_credits: 0.0,
            latency_ms: 100,
            outcome: "OK".to_owned(),
            error: None,
            reservation_id: Some("resv-f1".to_owned()),
        })?,
    ];
    fs::write(arm_ctrl_1.join("relay/calls.jsonl"), calls_lines.join("\n"))?;
    fs::write(
        arm_ctrl_1.join("relay/seats/Foundry.day-01-act.tool-round-0.json"),
        serde_json::to_string(&SavedToolRoundRecord {
            day: "01".to_owned(),
            round: Round::Act,
            guild: "Foundry".to_owned(),
            run_id: "run1".to_owned(),
            reservation_id: "resv-t0".to_owned(),
            request_body_sha256: "00".repeat(32),
            round_index: 0,
            attempt: 1,
            outcome: "TOOL_CALL".to_owned(),
            assistant_content: None,
            assistant_content_sha256: None,
            reasoning: Some("abcd".to_owned()),
            assistant_reply_sha256: "00".repeat(32),
            body_redacted: false,
            calls: Vec::new(),
        })?,
    )?;
    fs::write(
        arm_ctrl_1.join("relay/seats/Foundry.day-01-act.http-reply-round-0.json"),
        serde_json::to_string(&SavedHttpReplyRecord {
            day: "01".to_owned(),
            round: Round::Act,
            guild: "Foundry".to_owned(),
            run_id: "run1".to_owned(),
            reservation_id: "resv-t0".to_owned(),
            request_body_sha256: "00".repeat(32),
            round_index: 0,
            attempt: 1,
            status: 200,
            is_late: false,
            body_redacted: false,
            redacted_secret_field_paths: Vec::new(),
            body_sha256: "00".repeat(32),
            body: json!({"choices": [{"message": {"reasoning": "zzzz", "content": "{}"}}]})
                .to_string(),
            prompt_token_ids_sha256: None,
            prompt_token_ids_count: None,
            prompt_token_ids_source: None,
            prompt_token_ids_template_check: None,
        })?,
    )?;
    fs::write(
        arm_ctrl_1.join("relay/seats/Foundry.day-01-act.http-reply-round-1.json"),
        serde_json::to_string(&SavedHttpReplyRecord {
            day: "01".to_owned(),
            round: Round::Act,
            guild: "Foundry".to_owned(),
            run_id: "run1".to_owned(),
            reservation_id: "resv-f1".to_owned(),
            request_body_sha256: "00".repeat(32),
            round_index: 1,
            attempt: 1,
            status: 200,
            is_late: false,
            body_redacted: false,
            redacted_secret_field_paths: Vec::new(),
            body_sha256: "00".repeat(32),
            body: json!({"choices": [{"message": {"reasoning": null, "reasoning_content": "123456", "content": "{}"}}]})
                .to_string(),
            prompt_token_ids_sha256: None,
            prompt_token_ids_count: None,
            prompt_token_ids_source: None,
            prompt_token_ids_template_check: None,
        })?,
    )?;
    fs::write(
        arm_ctrl_1.join("relay/seats/Foundry.day-01-act.http-reply-round-2.json"),
        serde_json::to_string(&SavedHttpReplyRecord {
            day: "01".to_owned(),
            round: Round::Act,
            guild: "Foundry".to_owned(),
            run_id: "run1".to_owned(),
            reservation_id: "resv-f1".to_owned(),
            request_body_sha256: "00".repeat(32),
            round_index: 2,
            attempt: 1,
            status: 500,
            is_late: false,
            body_redacted: false,
            redacted_secret_field_paths: Vec::new(),
            body_sha256: "00".repeat(32),
            body: json!({"choices": [{"message": {"reasoning": "IGNORED"}}]}).to_string(),
            prompt_token_ids_sha256: None,
            prompt_token_ids_count: None,
            prompt_token_ids_source: None,
            prompt_token_ids_template_check: None,
        })?,
    )?;

    // On arm State 1: single tool call, refused_bundle_legs = 1 (without refused_bundle), negative revision gain.
    let mut dec_on = base_decision_record(EvalArm::On, 1, 0, "Foundry");
    dec_on.tool_calls_used = 1;
    dec_on.completion_tokens = 20;
    dec_on.refused_bundle_legs = 1;
    let out_on = BranchArmOutcome {
        source_game_id: "g_m14".to_owned(),
        table_id: 9200,
        day: 1,
        focal_seat: 0,
        focal_guild: "Foundry".to_owned(),
        communication: CommunicationMode::Talk,
        arm: EvalArm::On,
        decision: dec_on.clone(),
        receipt: default_receipt(1),
    };
    let (dec_on_p, out_on_p) =
        write_arm_outcome_pair(&tmp.path().join("arm_on"), &dec_on, &out_on)?;

    // Flat state (State 2) on control arm with empty relay/seats dir and no calls.jsonl ->
    // revision_gain_score_norm == None, sidecar chars == None.
    let arm_ctrl_2 = tmp.path().join("arm_ctrl_2");
    let mut dec_ctrl_2 = base_decision_record(EvalArm::Control, 1, 0, "Foundry");
    dec_ctrl_2.completion_tokens = 10;
    let out_ctrl_2 = BranchArmOutcome {
        source_game_id: "g_m14".to_owned(),
        table_id: 9200,
        day: 1,
        focal_seat: 0,
        focal_guild: "Foundry".to_owned(),
        communication: CommunicationMode::Talk,
        arm: EvalArm::Control,
        decision: dec_ctrl_2.clone(),
        receipt: default_receipt(1),
    };
    let (dec_c2_p, out_c2_p) = write_arm_outcome_pair(&arm_ctrl_2, &dec_ctrl_2, &out_ctrl_2)?;
    fs::create_dir_all(arm_ctrl_2.join("relay/seats"))?;

    let blind_dir = tmp.path().join("blind");
    let arm_map_dir = tmp.path().join("arm_map");
    let scores_dir = tmp.path().join("scores");
    let cfg = ScorerConfig::default();

    build_blind_and_score_files(
        &blind_dir,
        &arm_map_dir,
        &scores_dir,
        "g_m14-day-01-seat-0",
        "g_m14",
        1,
        0,
        &save_sha,
        &seal_sha,
        &pair_sha,
        &cfg,
        vec![
            ArmMapEntry {
                arm: EvalArm::Off,
                replicate: 0,
                committed_candidate_id: c0.clone(),
                first_draft_candidate_id: None,
                decision_path: dec_off_p,
                outcome_path: out_off_p,
                setup_path: Some(src_dir.join("setup.json")),
                manifest_path: Some(src_dir.join("manifest.json")),
            },
            ArmMapEntry {
                arm: EvalArm::Control,
                replicate: 0,
                committed_candidate_id: c2.clone(),
                first_draft_candidate_id: Some(c1.clone()),
                decision_path: dec_c1_p,
                outcome_path: out_c1_p,
                setup_path: Some(src_dir.join("setup.json")),
                manifest_path: Some(src_dir.join("manifest.json")),
            },
            ArmMapEntry {
                arm: EvalArm::On,
                replicate: 0,
                committed_candidate_id: c0.clone(),
                first_draft_candidate_id: Some(c2),
                decision_path: dec_on_p,
                outcome_path: out_on_p,
                setup_path: Some(src_dir.join("setup.json")),
                manifest_path: Some(src_dir.join("manifest.json")),
            },
        ],
        vec![
            (s0.clone(), 10.0, Some(0.5)),
            (s1.clone(), 5.0, Some(0.25)),
            (s2, 15.0, Some(0.75)),
        ],
        false,
    )?;

    build_blind_and_score_files(
        &blind_dir,
        &arm_map_dir,
        &scores_dir,
        "g_m14-day-01-seat-0-flat",
        "g_m14",
        1,
        0,
        &save_sha,
        &seal_sha,
        &pair_sha,
        &cfg,
        vec![ArmMapEntry {
            arm: EvalArm::Control,
            replicate: 0,
            committed_candidate_id: c1,
            first_draft_candidate_id: Some(c0),
            decision_path: dec_c2_p,
            outcome_path: out_c2_p,
            setup_path: Some(src_dir.join("setup.json")),
            manifest_path: Some(src_dir.join("manifest.json")),
        }],
        vec![(s0, 12.0, None), (s1, 14.0, None)],
        true,
    )?;

    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;
    let vc_path = verify_source_ckpts(&src_dir, &tmp.path().join("vc.json"))?;
    let freeze_path = tmp.path().join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &clean_git_sha_source(),
    )?;

    let out_dir = tmp.path().join("joined");
    run_join(&JoinOptions {
        freeze_path,
        arm_map_dir,
        scores_dir,
        out_dir: out_dir.clone(),
        verified_checkpoints_path: Some(vc_path),
        blind_dir: Some(blind_dir),
        source_game_dirs: Vec::new(),
    })?;

    let joined: JoinedDecisionsFile =
        serde_json::from_slice(&fs::read(out_dir.join("joined_decisions.json"))?)?;
    assert_eq!(
        joined.inferential_status,
        DESCRIPTIVE_ONLY_INFERENTIAL_STATUS
    );

    let off_row = joined
        .rows
        .iter()
        .find(|r| r.arm == EvalArm::Off)
        .ok_or("missing expected item")?;
    assert_eq!(off_row.mechanisms.tool_calls_used, 0);
    assert!(!off_row.mechanisms.used_any_tool_call);
    assert!(off_row.mechanisms.no_reply);
    assert_eq!(off_row.mechanisms.revision_gain_ev_total_equity, None);
    assert_eq!(off_row.mechanisms.revision_gain_score_norm, None);
    assert_eq!(off_row.mechanisms.total_reasoning_chars, None);
    assert_eq!(off_row.mechanisms.completion_tokens, 14);
    assert_eq!(off_row.mechanisms.reply_token_ids_len, None);
    assert!(off_row.mechanisms.has_reply_refusal);
    assert_eq!(off_row.mechanisms.refused_market_actions, 2);
    assert!(off_row.mechanisms.has_bundle_rejection);

    let c1_row = joined
        .rows
        .iter()
        .find(|r| r.arm == EvalArm::Control && r.state_id == "g_m14-day-01-seat-0")
        .ok_or("missing expected item")?;
    assert_eq!(c1_row.mechanisms.tool_calls_used, 2);
    assert!(c1_row.mechanisms.used_any_tool_call);
    assert_eq!(c1_row.mechanisms.revision_gain_ev_total_equity, Some(10.0));
    assert_eq!(c1_row.mechanisms.revision_gain_score_norm, Some(0.5));
    assert_eq!(c1_row.mechanisms.tool_round_reasoning_chars, Some(4));
    assert_eq!(c1_row.mechanisms.http_reply_reasoning_chars, Some(10));
    assert_eq!(c1_row.mechanisms.total_reasoning_chars, Some(10));
    assert_eq!(c1_row.mechanisms.completion_tokens, 30);
    assert_eq!(c1_row.mechanisms.reply_token_ids_len, Some(5));

    let c2_row = joined
        .rows
        .iter()
        .find(|r| r.arm == EvalArm::Control && r.state_id == "g_m14-day-01-seat-0-flat")
        .ok_or("missing expected item")?;
    assert_eq!(c2_row.mechanisms.revision_gain_ev_total_equity, Some(2.0));
    assert_eq!(c2_row.mechanisms.revision_gain_score_norm, None);
    assert_eq!(c2_row.mechanisms.tool_round_reasoning_chars, None);
    assert_eq!(c2_row.mechanisms.http_reply_reasoning_chars, None);
    assert_eq!(c2_row.mechanisms.total_reasoning_chars, None);
    assert_eq!(c2_row.mechanisms.unparseable_sidecars, 0);

    let on_row = joined
        .rows
        .iter()
        .find(|r| r.arm == EvalArm::On)
        .ok_or("missing expected item")?;
    assert!(on_row.mechanisms.used_any_tool_call);
    assert_eq!(on_row.mechanisms.revision_gain_ev_total_equity, Some(-5.0));
    assert_eq!(on_row.mechanisms.revision_gain_score_norm, Some(-0.25));
    assert!(on_row.mechanisms.has_bundle_rejection);

    let summary: PerArmSummaryFile =
        serde_json::from_slice(&fs::read(out_dir.join("per_arm_summary.json"))?)?;
    assert_eq!(
        summary.data_limits,
        MECHANISM_DATA_LIMITS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    );
    let ctrl_sum = summary
        .arms
        .iter()
        .find(|a| a.arm == EvalArm::Control)
        .ok_or("missing expected item")?;
    assert_eq!(ctrl_sum.mechanisms.n_decisions, 2);
    assert_eq!(ctrl_sum.mechanisms.n_with_tool_call, 1);
    assert_eq!(ctrl_sum.mechanisms.mean_tool_calls_used, Some(1.0));
    assert_eq!(ctrl_sum.mechanisms.tool_call_rate, Some(0.5));
    assert_eq!(ctrl_sum.mechanisms.n_with_first_draft, 2);
    assert_eq!(ctrl_sum.mechanisms.n_with_revision_gain_score_norm, 1);
    assert_eq!(
        ctrl_sum.mechanisms.mean_revision_gain_ev_total_equity,
        Some(6.0)
    );
    assert_eq!(ctrl_sum.mechanisms.mean_revision_gain_score_norm, Some(0.5));
    assert_eq!(ctrl_sum.mechanisms.n_with_reasoning_sidecars, 1);
    assert_eq!(ctrl_sum.mechanisms.n_with_reply_token_ids, 1);
    assert_eq!(ctrl_sum.mechanisms.mean_total_reasoning_chars, Some(10.0));
    assert_eq!(ctrl_sum.mechanisms.mean_reply_token_ids_len, Some(5.0));
    assert_eq!(ctrl_sum.mechanisms.n_unparseable_sidecars, 0);
    assert_eq!(ctrl_sum.mechanisms.n_with_pre_act_pledge_snapshot, 0);
    assert_eq!(ctrl_sum.mechanisms.n_with_terminal_scoreboard, 0);
    assert_eq!(ctrl_sum.mechanisms.n_with_talk_commitment_inputs, 0);
    assert_eq!(ctrl_sum.mechanisms.honour_rate, None);
    assert_eq!(ctrl_sum.mechanisms.focal_talk_counts, None);

    let empty_summary = summarize_arm_mechanisms(&[]);
    assert_eq!(empty_summary.n_decisions, 0);
    assert_eq!(empty_summary.tool_call_rate, None);
    assert_eq!(empty_summary.honour_rate, None);
    assert_eq!(empty_summary.mean_completion_tokens, None);
    Ok(())
}

#[test]
fn measure_5a_pledges_and_5b_deal_keeping_cover_all_buckets() -> TestResult {
    let tmp = TmpDir::new("m5_deal_keeping")?;
    let mut host = EvalHost::from_standard_setup(TableId(9201), 4, 2, 22)?;
    let mut t1_subs = BTreeMap::new();
    t1_subs.insert(
        0,
        EvalActSubmission {
            market_actions: vec![MarketAction::RecordIntentPledge {
                seat: 0,
                pledge: league_seat_types::IntentPledge {
                    pledge_id: 1,
                    promisor: 0,
                    counterparty: 1,
                    target: league_seat_types::IntentTarget::VoteForSeat {
                        office_turn: 4,
                        candidate: 1,
                    },
                    honor_weight: 50,
                    memo_digest: [1; 32],
                },
            }],
            ..EvalActSubmission::default()
        },
    );
    t1_subs.insert(
        1,
        EvalActSubmission {
            market_actions: vec![MarketAction::RecordIntentPledge {
                seat: 1,
                pledge: league_seat_types::IntentPledge {
                    pledge_id: 2,
                    promisor: 1,
                    counterparty: 2,
                    target: league_seat_types::IntentTarget::VoteForSeat {
                        office_turn: 4,
                        candidate: 2,
                    },
                    honor_weight: 50,
                    memo_digest: [2; 32],
                },
            }],
            ..EvalActSubmission::default()
        },
    );
    let _ = host.apply_act_submissions(&t1_subs)?;
    let _ = host.sweep_turn()?;

    let src_dir = tmp.path().join("src_5");
    let (save_sha, seal_sha, pair_sha) =
        write_minimal_source_game(&src_dir, &mut host, EvalArm::Control, "g_m5")?;

    let obs0 = host.observe_seat(Seat(0))?;
    let raw0_bal = obs0.seats[0].raw[0];
    let cred_bal = obs0.seats[0].cred;
    assert!(raw0_bal >= 5 * UNIT_SCALE);
    assert!(cred_bal >= 5 * UNIT_SCALE);
    let impossible_units = (raw0_bal / UNIT_SCALE) + 1000;
    fs::write(
        src_dir.join("decisions/day-02-talk-Foundry.json"),
        to_sorted_pretty_json(&EvalDecisionRecord {
            raw_reply: Some(
                json!({
                    "messages": [
                        { "to_guild": "Quarry", "body": "I will sell 2 raw 0 at 10." },
                        { "to_guild": "Hearth", "body": "I will buy 2 raw 1 at 2." },
                        { "to_guild": "Wharf", "body": format!("I will sell {impossible_units} raw 0 at 10.") }
                    ],
                    "market_actions": [],
                    "bundle": {},
                    "notes": "talk"
                })
                .to_string(),
            ),
            ..base_decision_record(EvalArm::Control, 2, 0, "Foundry")
        })?,
    )?;
    fs::write(
        src_dir.join("to-seat/day-02-act-Quarry.negotiation.json"),
        to_sorted_pretty_json(&NegotiationInputs {
            batna: "b".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Foundry".to_owned(),
                intent_header: None,
                body: "Hi.".to_owned(),
            }],
        })?,
    )?;

    fs::write(
        src_dir.join("scoreboard.json"),
        to_sorted_pretty_json(&FinalScoreboard {
            table_id: TableId(9201),
            closed_turn: 2,
            standings: vec![SeatScoreEntry {
                seat: 0,
                project_equity: 0,
                net_asset_equity: 0,
                mandate_bonus: 0,
                total_equity: 0,
                credit_tier: league_seat_types::CreditTier::Aaa,
                hard_credit_defaults: 0,
                bond_breaches: 0,
                broken_promises: 3,
                honor_penalties: 0,
            }],
        })?,
    )?;

    let s0 = sample_submission(0);
    let c0 = candidate_id_for_submission(&s0)?;

    let mut rec_off = default_receipt(2);
    rec_off.broken_promises.push(BrokenPromiseRecord {
        pledge_id: 1,
        promisor: 0,
        counterparty: 1,
        weight: 50,
    });
    rec_off.broken_promises.push(BrokenPromiseRecord {
        pledge_id: 2,
        promisor: 1,
        counterparty: 2,
        weight: 50,
    });
    rec_off.fills = vec![
        FillRecord {
            turn: 2,
            kind: FillKind::ContractDelivery,
            good: Some(AssetId::Raw(0)),
            from: Some(0),
            to: 1,
            units: 2 * UNIT_SCALE,
            net_units: 2 * UNIT_SCALE,
            cred_amount: 0,
            unit_price_cred: None,
        },
        FillRecord {
            turn: 2,
            kind: FillKind::BatchFill,
            good: Some(AssetId::Raw(0)),
            from: Some(0),
            to: 1,
            units: 2 * UNIT_SCALE,
            net_units: 2 * UNIT_SCALE,
            cred_amount: 20 * UNIT_SCALE,
            unit_price_cred: Some(10 * UNIT_SCALE),
        },
        FillRecord {
            turn: 2,
            kind: FillKind::AtomicSwap,
            good: Some(AssetId::Raw(1)),
            from: Some(2),
            to: 0,
            units: 2 * UNIT_SCALE,
            net_units: 2 * UNIT_SCALE,
            cred_amount: 6 * UNIT_SCALE,
            unit_price_cred: Some(3 * UNIT_SCALE),
        },
    ];
    let dec_off = base_decision_record(EvalArm::Off, 2, 0, "Foundry");
    let (dec_off_p, out_off_p) = write_arm_outcome_pair(
        &tmp.path().join("m5_off"),
        &dec_off,
        &BranchArmOutcome {
            source_game_id: "g_m5".to_owned(),
            table_id: 9201,
            day: 2,
            focal_seat: 0,
            focal_guild: "Foundry".to_owned(),
            communication: CommunicationMode::Talk,
            arm: EvalArm::Off,
            decision: dec_off.clone(),
            receipt: rec_off,
        },
    )?;

    let mut rec_ctrl = default_receipt(2);
    rec_ctrl.fills = vec![FillRecord {
        turn: 2,
        kind: FillKind::BatchFill,
        good: Some(AssetId::Raw(0)),
        from: Some(0),
        to: 1,
        units: UNIT_SCALE,
        net_units: UNIT_SCALE,
        cred_amount: 10 * UNIT_SCALE,
        unit_price_cred: Some(10 * UNIT_SCALE),
    }];
    let mut dec_ctrl = base_decision_record(EvalArm::Control, 2, 0, "Foundry");
    dec_ctrl.raw_reply = Some(
        json!({
            "messages": [],
            "market_actions": [
                {
                    "action_kind": "post_offer",
                    "side": "sell",
                    "good": { "asset_kind": "raw", "good_index": 0 },
                    "amount": 2 * UNIT_SCALE,
                    "price_per_unit_cred": 10 * UNIT_SCALE
                },
                {
                    "action_kind": "post_offer",
                    "side": "buy",
                    "good": { "asset_kind": "raw", "good_index": 1 },
                    "amount": 2 * UNIT_SCALE,
                    "price_per_unit_cred": 2 * UNIT_SCALE
                }
            ],
            "bundle": {},
            "notes": "act"
        })
        .to_string(),
    );
    dec_ctrl.apply_report = Some(EvalSeatApplyReport {
        refused_market_actions: vec![(0, "TEST_REFUSED".to_owned())],
        refused_pass_b_actions: Vec::new(),
        refused_bundle_code: None,
    });
    let (dec_ctrl_p, out_ctrl_p) = write_arm_outcome_pair(
        &tmp.path().join("m5_ctrl"),
        &dec_ctrl,
        &BranchArmOutcome {
            source_game_id: "g_m5".to_owned(),
            table_id: 9201,
            day: 2,
            focal_seat: 0,
            focal_guild: "Foundry".to_owned(),
            communication: CommunicationMode::Talk,
            arm: EvalArm::Control,
            decision: dec_ctrl.clone(),
            receipt: rec_ctrl,
        },
    )?;

    let mut rec_on = default_receipt(2);
    rec_on.fills = vec![FillRecord {
        turn: 2,
        kind: FillKind::BatchFill,
        good: Some(AssetId::Raw(0)),
        from: Some(0),
        to: 3,
        units: 2 * UNIT_SCALE,
        net_units: 2 * UNIT_SCALE,
        cred_amount: 20 * UNIT_SCALE,
        unit_price_cred: Some(10 * UNIT_SCALE),
    }];
    let dec_on = base_decision_record(EvalArm::On, 2, 0, "Foundry");
    let (dec_on_p, out_on_p) = write_arm_outcome_pair(
        &tmp.path().join("m5_on"),
        &dec_on,
        &BranchArmOutcome {
            source_game_id: "g_m5".to_owned(),
            table_id: 9201,
            day: 2,
            focal_seat: 0,
            focal_guild: "Foundry".to_owned(),
            communication: CommunicationMode::Talk,
            arm: EvalArm::On,
            decision: dec_on.clone(),
            receipt: rec_on,
        },
    )?;

    let blind_dir = tmp.path().join("blind");
    let arm_map_dir = tmp.path().join("arm_map");
    let scores_dir = tmp.path().join("scores");
    let cfg = ScorerConfig::default();

    build_blind_and_score_files(
        &blind_dir,
        &arm_map_dir,
        &scores_dir,
        "g_m5-day-02-seat-0",
        "g_m5",
        2,
        0,
        &save_sha,
        &seal_sha,
        &pair_sha,
        &cfg,
        vec![
            ArmMapEntry {
                arm: EvalArm::Off,
                replicate: 0,
                committed_candidate_id: c0.clone(),
                first_draft_candidate_id: None,
                decision_path: dec_off_p,
                outcome_path: out_off_p,
                setup_path: Some(src_dir.join("setup.json")),
                manifest_path: Some(src_dir.join("manifest.json")),
            },
            ArmMapEntry {
                arm: EvalArm::Control,
                replicate: 0,
                committed_candidate_id: c0.clone(),
                first_draft_candidate_id: None,
                decision_path: dec_ctrl_p,
                outcome_path: out_ctrl_p,
                setup_path: Some(src_dir.join("setup.json")),
                manifest_path: Some(src_dir.join("manifest.json")),
            },
            ArmMapEntry {
                arm: EvalArm::On,
                replicate: 0,
                committed_candidate_id: c0,
                first_draft_candidate_id: None,
                decision_path: dec_on_p,
                outcome_path: out_on_p,
                setup_path: Some(src_dir.join("setup.json")),
                manifest_path: Some(src_dir.join("manifest.json")),
            },
        ],
        vec![(s0, 10.0, Some(0.5))],
        false,
    )?;

    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;
    let vc_path = verify_source_ckpts(&src_dir, &tmp.path().join("vc_m5a.json"))?;
    let freeze_path = tmp.path().join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &clean_git_sha_source(),
    )?;

    let out_dir = tmp.path().join("joined");
    run_join(&JoinOptions {
        freeze_path,
        arm_map_dir,
        scores_dir,
        out_dir: out_dir.clone(),
        verified_checkpoints_path: Some(vc_path),
        blind_dir: Some(blind_dir),
        source_game_dirs: vec![src_dir],
    })?;

    let joined: JoinedDecisionsFile =
        serde_json::from_slice(&fs::read(out_dir.join("joined_decisions.json"))?)?;
    let off = joined
        .rows
        .iter()
        .find(|r| r.arm == EvalArm::Off)
        .ok_or("missing expected item")?;
    assert_eq!(off.mechanisms.pre_act_pledges_count, Some(1));
    assert_eq!(off.mechanisms.pre_act_audited_pledges_count, Some(0));
    assert_eq!(off.mechanisms.sweep_broken_promises_count, 1);
    assert_eq!(off.mechanisms.terminal_broken_promises_count, Some(3));
    assert_eq!(off.mechanisms.extracted_outgoing_commitments, Some(3));
    assert_eq!(off.mechanisms.applicable_commitments, Some(2));
    assert_eq!(off.mechanisms.inapplicable_commitments, Some(1));
    assert_eq!(off.mechanisms.honoured_commitments, Some(2));
    assert_eq!(off.mechanisms.honour_rate, Some(1.0));
    assert_eq!(
        off.mechanisms.focal_talk_counts.as_ref().map(|c| (
            c.negotiation.seen,
            c.talk.seen,
            c.total.matched
        )),
        Some((1, 3, 4))
    );

    let ctrl = joined
        .rows
        .iter()
        .find(|r| r.arm == EvalArm::Control)
        .ok_or("missing expected item")?;
    assert_eq!(ctrl.mechanisms.honoured_commitments, Some(0));
    assert_eq!(ctrl.mechanisms.partial_bilateral_fill_commitments, Some(1));
    assert_eq!(ctrl.mechanisms.posted_unfilled_commitments, Some(1));
    assert_eq!(ctrl.mechanisms.honour_rate, Some(0.0));

    let on = joined
        .rows
        .iter()
        .find(|r| r.arm == EvalArm::On)
        .ok_or("missing expected item")?;
    assert_eq!(on.mechanisms.honoured_commitments, Some(0));
    assert_eq!(on.mechanisms.third_party_fill_commitments, Some(1));
    assert_eq!(on.mechanisms.posted_unfilled_commitments, Some(0));

    let summary: PerArmSummaryFile =
        serde_json::from_slice(&fs::read(out_dir.join("per_arm_summary.json"))?)?;
    let off_sum = summary
        .arms
        .iter()
        .find(|a| a.arm == EvalArm::Off)
        .ok_or("missing expected item")?;
    assert_eq!(off_sum.mechanisms.n_with_talk_commitment_inputs, 1);
    assert_eq!(off_sum.mechanisms.honour_rate, Some(1.0));
    assert_eq!(
        off_sum
            .mechanisms
            .focal_talk_counts
            .as_ref()
            .map(|t| t.totals.total.matched),
        Some(4)
    );
    Ok(())
}

fn make_call_log_entry(
    day: &str,
    round: Round,
    guild: &str,
    outcome: &str,
    resv_id: &str,
) -> CallLogEntry {
    CallLogEntry {
        time_utc: "2026-11-23T00:00:00Z".to_owned(),
        day: day.to_owned(),
        round,
        guild: guild.to_owned(),
        attempt: 1,
        trigger: "turn".to_owned(),
        model_requested: "m".to_owned(),
        model_returned: Some("m".to_owned()),
        request_body_sha256: "00".repeat(32),
        http_reply_sha256: None,
        reply_sha256: None,
        prompt_tokens: Some(10),
        completion_tokens: Some(10),
        total_tokens: Some(20),
        reasoning_tokens: None,
        cost_unit: league_relay::CostUnit::default(),
        wall_secs: Some(0.1),
        cost: None,
        charged_credits: 0.0,
        latency_ms: 100,
        outcome: outcome.to_owned(),
        error: None,
        reservation_id: Some(resv_id.to_owned()),
    }
}

fn make_tool_round_record(
    day: &str,
    guild: &str,
    resv_id: &str,
    round_index: usize,
    reasoning: Option<&str>,
) -> SavedToolRoundRecord {
    SavedToolRoundRecord {
        day: day.to_owned(),
        round: Round::Act,
        guild: guild.to_owned(),
        run_id: "r".to_owned(),
        reservation_id: resv_id.to_owned(),
        request_body_sha256: "00".repeat(32),
        round_index,
        attempt: 1,
        outcome: "TOOL_CALL".to_owned(),
        assistant_content: None,
        assistant_content_sha256: None,
        reasoning: reasoning.map(str::to_owned),
        assistant_reply_sha256: "00".repeat(32),
        body_redacted: false,
        calls: Vec::new(),
    }
}

fn make_http_reply_record(
    day: &str,
    guild: &str,
    resv_id: &str,
    round_index: usize,
    status: u16,
    body: String,
) -> SavedHttpReplyRecord {
    SavedHttpReplyRecord {
        day: day.to_owned(),
        round: Round::Act,
        guild: guild.to_owned(),
        run_id: "r".to_owned(),
        reservation_id: resv_id.to_owned(),
        request_body_sha256: "00".repeat(32),
        round_index,
        attempt: 1,
        status,
        is_late: false,
        body_redacted: false,
        redacted_secret_field_paths: Vec::new(),
        body_sha256: "00".repeat(32),
        body,
        prompt_token_ids_sha256: None,
        prompt_token_ids_count: None,
        prompt_token_ids_source: None,
        prompt_token_ids_template_check: None,
    }
}

fn find_sentinel_hits_recursive(
    root: &Path,
    needle: &str,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut hits = Vec::new();
    if !root.exists() {
        return Ok(hits);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<PathBuf> = fs::read_dir(&dir)?
            .map(|res| res.map(|e| e.path()))
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort();
        for path in entries {
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                let bytes = fs::read(&path)?;
                let text = String::from_utf8_lossy(&bytes);
                if text.contains(needle) {
                    hits.push(path);
                }
            }
        }
    }
    Ok(hits)
}

/// Test-only unguarded serde parser proving that malformed inputs quote sentinel strings if parsed unsafely.
fn leaky_serde_parse_control<T: DeserializeOwned>(raw: &str) -> Result<T, String> {
    serde_json::from_str::<T>(raw).map_err(|e| e.to_string())
}

#[test]
fn sentinels_never_leak_to_files_stdout_or_stderr_on_success_or_error_paths() -> TestResult {
    let tmp = TmpDir::new("sentinel_noleak")?;
    let mut host = EvalHost::from_standard_setup(TableId(9202), 4, 1, 33)?;
    let src_dir = tmp.path().join("src_sec");
    let (save_sha, seal_sha, pair_sha) =
        write_minimal_source_game(&src_dir, &mut host, EvalArm::Control, "g_sent")?;

    let sentinel = "PRIVATE_REASONING_WHISPER_SENTINEL_7X9Q";
    let talk_dec_path = src_dir.join("decisions/day-01-talk-Foundry.json");
    let orig_talk_dec = to_sorted_pretty_json(&EvalDecisionRecord {
        raw_reply: Some(
            json!({
                "messages": [{ "to_guild": "Quarry", "body": format!("Hello {sentinel}?") }],
                "market_actions": [],
                "bundle": {},
                "notes": "talk"
            })
            .to_string(),
        ),
        ..base_decision_record(EvalArm::Control, 1, 0, "Foundry")
    })?;
    fs::write(&talk_dec_path, &orig_talk_dec)?;
    let neg_path = src_dir.join("to-seat/day-01-act-Quarry.negotiation.json");
    let orig_neg = to_sorted_pretty_json(&NegotiationInputs {
        batna: "b".to_owned(),
        whispers: vec![UntrustedWhisper {
            from_guild: "Foundry".to_owned(),
            intent_header: None,
            body: format!("Ambiguous {sentinel}?"),
        }],
    })?;
    fs::write(&neg_path, &orig_neg)?;

    let arm_dir = tmp.path().join("arm_ctrl");
    let dec = base_decision_record(EvalArm::Control, 1, 0, "Foundry");
    let (dec_p, out_p) = write_arm_outcome_pair(
        &arm_dir,
        &dec,
        &BranchArmOutcome {
            source_game_id: "g_sent".to_owned(),
            table_id: 9202,
            day: 1,
            focal_seat: 0,
            focal_guild: "Foundry".to_owned(),
            communication: CommunicationMode::Talk,
            arm: EvalArm::Control,
            decision: dec.clone(),
            receipt: default_receipt(1),
        },
    )?;
    let orig_dec = fs::read_to_string(&dec_p)?;
    fs::create_dir_all(arm_dir.join("relay/seats"))?;
    let calls_path = arm_dir.join("relay/calls.jsonl");
    let orig_calls = format!(
        "{}\n{}\n",
        serde_json::to_string(&make_call_log_entry(
            "01",
            Round::Act,
            "Foundry",
            "TOOL_CALL",
            "r0"
        ))?,
        serde_json::to_string(&make_call_log_entry(
            "01",
            Round::Act,
            "Foundry",
            "OK",
            "r1"
        ))?,
    );
    fs::write(&calls_path, &orig_calls)?;
    let tool_path = arm_dir.join("relay/seats/Foundry.day-01-act.tool-round-0.json");
    let orig_tool = serde_json::to_string(&make_tool_round_record(
        "01",
        "Foundry",
        "r0",
        0,
        Some(&format!("tool-{sentinel}")),
    ))?;
    fs::write(&tool_path, &orig_tool)?;
    let http_path = arm_dir.join("relay/seats/Foundry.day-01-act.http-reply-round-1.json");
    let orig_http = serde_json::to_string(&make_http_reply_record(
        "01",
        "Foundry",
        "r1",
        1,
        200,
        json!({"choices": [{"message": {"reasoning": format!("http-{sentinel}"), "content": "{}"}}]}).to_string(),
    ))?;
    fs::write(&http_path, &orig_http)?;

    let counts = measure_reasoning_length_in_memory(&arm_dir, 1, "Foundry")?;
    assert_eq!(
        counts.tool_round_reasoning_chars,
        Some(format!("tool-{sentinel}").chars().count() as u64)
    );
    assert_eq!(
        counts.http_reply_reasoning_chars,
        Some(format!("http-{sentinel}").chars().count() as u64)
    );

    let blind_dir = tmp.path().join("blind");
    let arm_map_dir = tmp.path().join("arm_map");
    let scores_dir = tmp.path().join("scores");
    let cfg = ScorerConfig::default();
    let s0 = sample_submission(0);
    let c0 = candidate_id_for_submission(&s0)?;
    build_blind_and_score_files(
        &blind_dir,
        &arm_map_dir,
        &scores_dir,
        "g_sent-day-01-seat-0",
        "g_sent",
        1,
        0,
        &save_sha,
        &seal_sha,
        &pair_sha,
        &cfg,
        vec![ArmMapEntry {
            arm: EvalArm::Control,
            replicate: 0,
            committed_candidate_id: c0,
            first_draft_candidate_id: None,
            decision_path: dec_p.clone(),
            outcome_path: out_p,
            setup_path: Some(src_dir.join("setup.json")),
            manifest_path: Some(src_dir.join("manifest.json")),
        }],
        vec![(s0, 10.0, Some(0.5))],
        false,
    )?;

    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;
    let vc_path = verify_source_ckpts(&src_dir, &tmp.path().join("vc_sent.json"))?;
    let freeze_path = tmp.path().join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &clean_git_sha_source(),
    )?;

    let outputs_root = tmp.path().join("all_outputs");
    fs::create_dir_all(&outputs_root)?;
    let git_sha = clean_git_sha_source();
    let vc_str = vc_path.to_str().unwrap_or_default().to_owned();
    let run_cli_join = |out_name: &str, with_src: bool| -> (PathBuf, i32, String, String) {
        let out_dir = outputs_root.join(out_name);
        let mut args = vec![
            "--error-format",
            "json",
            "join",
            "--freeze",
            freeze_path.to_str().unwrap_or_default(),
            "--arm-map",
            arm_map_dir.to_str().unwrap_or_default(),
            "--scores",
            scores_dir.to_str().unwrap_or_default(),
            "--verified-checkpoints",
            vc_str.as_str(),
            "--out",
            out_dir.to_str().unwrap_or_default(),
        ];
        if with_src {
            args.push("--source-game");
            args.push(src_dir.to_str().unwrap_or_default());
        }
        let (c, so, se) = run_eval_cli(&args, &git_sha);
        (out_dir, c, so, se)
    };

    // 1) Full CLI join success path.
    let (out_ok, code, stdout, stderr) = run_cli_join("joined_ok", true);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(!stdout.contains(sentinel));
    assert!(!stderr.contains(sentinel));
    assert!(find_sentinel_hits_recursive(&out_ok, sentinel)?.is_empty());

    // 2) Five malformed sidecars where serde quotes the sentinel on type/enum errors, each run
    // independently, plus a malformed required decision.json failure run.
    let bad_tool = json!({"day": "01", "round": sentinel}).to_string();
    let bad_http = json!({"day": "01", "status": sentinel}).to_string();
    let bad_calls = json!({"day": "01", "round": sentinel}).to_string();
    let bad_neg = json!({"batna": "b", "whispers": sentinel}).to_string();
    let bad_talk = json!({"day": sentinel}).to_string();
    let bad_dec = json!({"round": sentinel}).to_string();

    // Positive control on payloads: verify naive serde formatting genuinely quotes sentinel.
    assert!(
        leaky_serde_parse_control::<SavedToolRoundRecord>(&bad_tool)
            .is_err_and(|e| e.contains(sentinel))
    );
    assert!(
        leaky_serde_parse_control::<SavedHttpReplyRecord>(&bad_http)
            .is_err_and(|e| e.contains(sentinel))
    );
    assert!(
        leaky_serde_parse_control::<CallLogEntry>(&bad_calls).is_err_and(|e| e.contains(sentinel))
    );
    assert!(
        leaky_serde_parse_control::<NegotiationInputs>(&bad_neg)
            .is_err_and(|e| e.contains(sentinel))
    );
    assert!(
        leaky_serde_parse_control::<EvalDecisionRecord>(&bad_talk)
            .is_err_and(|e| e.contains(sentinel))
    );
    assert!(
        leaky_serde_parse_control::<EvalDecisionRecord>(&bad_dec)
            .is_err_and(|e| e.contains(sentinel))
    );

    let sidecar_runs: [(&str, &Path, &str, &str); 5] = [
        ("err_tool", &tool_path, &orig_tool, &bad_tool),
        ("err_http", &http_path, &orig_http, &bad_http),
        ("err_calls", &calls_path, &orig_calls, &bad_calls),
        ("err_neg", &neg_path, &orig_neg, &bad_neg),
        ("err_talk", &talk_dec_path, &orig_talk_dec, &bad_talk),
    ];
    for (run_name, target_path, orig_content, bad_content) in sidecar_runs {
        fs::write(target_path, bad_content)?;
        let (out_dir, rc, so, se) = run_cli_join(run_name, true);
        assert_eq!(rc, 0, "run={run_name} stderr={se}");
        assert!(
            !so.contains(sentinel),
            "stdout leaked sentinel in {run_name}"
        );
        assert!(
            !se.contains(sentinel),
            "stderr leaked sentinel in {run_name}"
        );
        let sum: PerArmSummaryFile =
            serde_json::from_slice(&fs::read(out_dir.join("per_arm_summary.json"))?)?;
        assert!(
            sum.arms[0].mechanisms.n_unparseable_sidecars >= 1,
            "expected n_unparseable_sidecars >= 1 in {run_name}"
        );
        fs::write(target_path, orig_content)?;
    }

    // Required decision.json failing with sentinel on enum error must abort without leaking.
    fs::write(&dec_p, &bad_dec)?;
    let (_, c_dec_err, out_dec_err, err_dec_err) = run_cli_join("err_req_dec", false);
    assert_ne!(c_dec_err, 0);
    assert!(!out_dec_err.contains(sentinel));
    assert!(
        !err_dec_err.contains(sentinel),
        "stderr leaked sentinel: {err_dec_err}"
    );
    fs::write(&dec_p, &orig_dec)?;

    // Unknown flag & missing --source-game dir error checks.
    let (c_err3, _, _) = run_eval_cli(&["join", "--unknown-join-flag", "x"], &git_sha);
    assert_ne!(c_err3, 0);
    let err_missing_src = run_join(&JoinOptions {
        freeze_path,
        arm_map_dir,
        scores_dir,
        out_dir: outputs_root.join("err_missing_src"),
        verified_checkpoints_path: Some(vc_path),
        blind_dir: None,
        source_game_dirs: vec![tmp.path().join("missing_source_game_dir")],
    })
    .err()
    .ok_or("expected missing --source-game error")?;
    assert_eq!(err_missing_src.code(), "MISSING_SOURCE_SETUP");

    // Walk every output directory recursively and assert zero hits anywhere.
    let all_hits = find_sentinel_hits_recursive(&outputs_root, sentinel)?;
    assert!(all_hits.is_empty(), "sentinel hits found: {all_hits:?}");

    // Control check on recursive walker: a leaked error written in a nested subdir IS detected.
    let control_leak = tmp.path().join("control_leak/sub/nested_err.log");
    fs::create_dir_all(control_leak.parent().ok_or("missing parent")?)?;
    let leaked_msg = leaky_serde_parse_control::<SavedHttpReplyRecord>(&bad_http)
        .err()
        .ok_or("expected control parse error")?;
    fs::write(&control_leak, leaked_msg)?;
    let detected = find_sentinel_hits_recursive(&tmp.path().join("control_leak"), sentinel)?;
    assert_eq!(detected, vec![control_leak]);
    Ok(())
}

#[test]
fn measure_5b_cross_source_retraction_zero_applicable_and_duplicate_source_errors() -> TestResult {
    let tmp = TmpDir::new("m5_retract_zero")?;
    let mut host = EvalHost::from_standard_setup(TableId(9203), 4, 1, 44)?;
    let src_dir = tmp.path().join("src_ret");
    let (save_sha, seal_sha, pair_sha) =
        write_minimal_source_game(&src_dir, &mut host, EvalArm::Control, "g_ret")?;

    // Outgoing offer in decisions/day-01-talk-Foundry.json, retracted across source in to-seat/day-01-act-Quarry.negotiation.json.
    fs::write(
        src_dir.join("decisions/day-01-talk-Foundry.json"),
        to_sorted_pretty_json(&EvalDecisionRecord {
            raw_reply: Some(
                json!({
                    "messages": [{ "to_guild": "Quarry", "body": "I will sell 2 raw 0 at 10." }],
                    "market_actions": [],
                    "bundle": {},
                    "notes": "talk"
                })
                .to_string(),
            ),
            ..base_decision_record(EvalArm::Control, 1, 0, "Foundry")
        })?,
    )?;
    fs::write(
        src_dir.join("to-seat/day-01-act-Quarry.negotiation.json"),
        to_sorted_pretty_json(&NegotiationInputs {
            batna: "b".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Foundry".to_owned(),
                intent_header: None,
                body: "No.".to_owned(),
            }],
        })?,
    )?;

    let s0 = sample_submission(0);
    let c0 = candidate_id_for_submission(&s0)?;
    let dec = base_decision_record(EvalArm::Control, 1, 0, "Foundry");
    let (dec_p, out_p) = write_arm_outcome_pair(
        &tmp.path().join("arm_ret"),
        &dec,
        &BranchArmOutcome {
            source_game_id: "g_ret".to_owned(),
            table_id: 9203,
            day: 1,
            focal_seat: 0,
            focal_guild: "Foundry".to_owned(),
            communication: CommunicationMode::Talk,
            arm: EvalArm::Control,
            decision: dec.clone(),
            receipt: default_receipt(1),
        },
    )?;

    let blind_dir = tmp.path().join("blind");
    let arm_map_dir = tmp.path().join("arm_map");
    let scores_dir = tmp.path().join("scores");
    let cfg = ScorerConfig::default();
    build_blind_and_score_files(
        &blind_dir,
        &arm_map_dir,
        &scores_dir,
        "g_ret-day-01-seat-0",
        "g_ret",
        1,
        0,
        &save_sha,
        &seal_sha,
        &pair_sha,
        &cfg,
        vec![ArmMapEntry {
            arm: EvalArm::Control,
            replicate: 0,
            committed_candidate_id: c0,
            first_draft_candidate_id: None,
            decision_path: dec_p,
            outcome_path: out_p,
            setup_path: Some(src_dir.join("setup.json")),
            manifest_path: Some(src_dir.join("manifest.json")),
        }],
        vec![(s0, 10.0, Some(0.5))],
        false,
    )?;

    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;
    let vc_path = verify_source_ckpts(&src_dir, &tmp.path().join("vc_ret.json"))?;
    let freeze_path = tmp.path().join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &clean_git_sha_source(),
    )?;

    let out_dir = tmp.path().join("joined");
    run_join(&JoinOptions {
        freeze_path: freeze_path.clone(),
        arm_map_dir: arm_map_dir.clone(),
        scores_dir: scores_dir.clone(),
        out_dir: out_dir.clone(),
        verified_checkpoints_path: Some(vc_path.clone()),
        blind_dir: None,
        source_game_dirs: vec![src_dir.clone()],
    })?;

    let joined: JoinedDecisionsFile =
        serde_json::from_slice(&fs::read(out_dir.join("joined_decisions.json"))?)?;
    let m = &joined.rows[0].mechanisms;
    assert_eq!(m.extracted_outgoing_commitments, Some(0));
    assert_eq!(m.applicable_commitments, Some(0));
    assert_eq!(m.honoured_commitments, Some(0));
    assert_eq!(m.honour_rate, None);

    // Duplicate source game_id across distinct --source-game dirs must fail closed.
    let dup_dir = tmp.path().join("src_dup");
    fs::create_dir_all(&dup_dir)?;
    fs::write(
        dup_dir.join("setup.json"),
        fs::read(src_dir.join("setup.json"))?,
    )?;
    let dup_err = run_join(&JoinOptions {
        freeze_path: freeze_path.clone(),
        arm_map_dir: arm_map_dir.clone(),
        scores_dir: scores_dir.clone(),
        out_dir: tmp.path().join("joined_dup"),
        verified_checkpoints_path: Some(vc_path.clone()),
        blind_dir: None,
        source_game_dirs: vec![src_dir.clone(), dup_dir],
    })
    .err()
    .ok_or("expected duplicate game_id error")?;
    assert_eq!(dup_err.code(), "DUPLICATE_STATE_ID");

    // Pre-act checkpoint SHA mismatch refuses with FREEZE_MANIFEST_MISMATCH.
    let tamper_src = tmp.path().join("src_tamper");
    let mut host_tamper = EvalHost::from_standard_setup(TableId(9204), 4, 1, 55)?;
    let _ = write_minimal_source_game(&tamper_src, &mut host_tamper, EvalArm::Control, "g_ret")?;
    fs::write(
        tamper_src.join("checkpoints/day-01-pre-act.save.json"),
        b"{\"tampered\":true}",
    )?;
    let tamper_err = run_join(&JoinOptions {
        freeze_path,
        arm_map_dir,
        scores_dir,
        out_dir: tmp.path().join("joined_tamper"),
        verified_checkpoints_path: Some(vc_path),
        blind_dir: None,
        source_game_dirs: vec![tamper_src],
    })
    .err()
    .ok_or("expected tampered pre-act checkpoint error")?;
    assert_eq!(tamper_err.code(), "FREEZE_MANIFEST_MISMATCH");
    Ok(())
}

#[test]
fn measure_5b_neg_merge_recursive_source_scan_and_sidecar_filters() -> TestResult {
    let tmp = TmpDir::new("m5_neg_merge_filters")?;
    let mut host = EvalHost::from_standard_setup(TableId(9205), 4, 2, 66)?;
    let _ = host.apply_act_submissions(&BTreeMap::new())?;
    let _ = host.sweep_turn()?;
    let obs = host.observe_seat(Seat(0))?;
    assert!(obs.seats[0].refined[0] >= UNIT_SCALE || obs.seats[0].cred >= 2 * UNIT_SCALE);

    // Put source game nested under a parent dir passed to --source-game.
    let parent_src = tmp.path().join("source_root");
    let src_dir = parent_src.join("nested").join("game_merge");
    let (save_sha, seal_sha, pair_sha) =
        write_minimal_source_game(&src_dir, &mut host, EvalArm::Control, "g_m5_neg")?;

    // Offer 1 (raw 0): identical across negotiation.json and talk.json.
    // Offer 2 (raw 1): present only in negotiation.json, kept unretracted in talk.json.
    fs::write(
        src_dir.join("to-seat/day-02-act-Quarry.negotiation.json"),
        to_sorted_pretty_json(&NegotiationInputs {
            batna: "b".to_owned(),
            whispers: vec![
                UntrustedWhisper {
                    from_guild: "Foundry".to_owned(),
                    intent_header: None,
                    body: "I will sell 1 raw 0 at 10.".to_owned(),
                },
                UntrustedWhisper {
                    from_guild: "Foundry".to_owned(),
                    intent_header: None,
                    body: "I will buy 1 refined 0 at 2.".to_owned(),
                },
            ],
        })?,
    )?;
    fs::write(
        src_dir.join("decisions/day-02-talk-Foundry.json"),
        to_sorted_pretty_json(&EvalDecisionRecord {
            raw_reply: Some(
                json!({
                    "messages": [{ "to_guild": "Quarry", "body": "I will sell 1 raw 0 at 10." }],
                    "market_actions": [],
                    "bundle": {},
                    "notes": "talk"
                })
                .to_string(),
            ),
            ..base_decision_record(EvalArm::Control, 2, 0, "Foundry")
        })?,
    )?;

    let mut rec = default_receipt(2);
    // Include non-qualifying fills (wrong turn, EscrowFinished, worse price, third-party).
    rec.fills = vec![
        FillRecord {
            turn: 3,
            kind: FillKind::BatchFill,
            good: Some(AssetId::Raw(0)),
            from: Some(0),
            to: 1,
            units: UNIT_SCALE,
            net_units: UNIT_SCALE,
            cred_amount: 10 * UNIT_SCALE,
            unit_price_cred: Some(10 * UNIT_SCALE),
        },
        FillRecord {
            turn: 2,
            kind: FillKind::EscrowFinished,
            good: Some(AssetId::Raw(0)),
            from: Some(0),
            to: 1,
            units: UNIT_SCALE,
            net_units: UNIT_SCALE,
            cred_amount: 10 * UNIT_SCALE,
            unit_price_cred: Some(10 * UNIT_SCALE),
        },
        FillRecord {
            turn: 2,
            kind: FillKind::BatchFill,
            good: Some(AssetId::Raw(0)),
            from: Some(0),
            to: 1,
            units: UNIT_SCALE,
            net_units: UNIT_SCALE,
            cred_amount: 11 * UNIT_SCALE,
            unit_price_cred: Some(11 * UNIT_SCALE),
        },
        FillRecord {
            turn: 2,
            kind: FillKind::BatchFill,
            good: Some(AssetId::Refined(0)),
            from: Some(2),
            to: 0,
            units: UNIT_SCALE,
            net_units: UNIT_SCALE,
            cred_amount: 2 * UNIT_SCALE,
            unit_price_cred: Some(2 * UNIT_SCALE),
        },
    ];

    let arm_dir = tmp.path().join("arm_merge");
    let dec = base_decision_record(EvalArm::Control, 2, 0, "Foundry");
    let (dec_p, out_p) = write_arm_outcome_pair(
        &arm_dir,
        &dec,
        &BranchArmOutcome {
            source_game_id: "g_m5_neg".to_owned(),
            table_id: 9205,
            day: 2,
            focal_seat: 0,
            focal_guild: "Foundry".to_owned(),
            communication: CommunicationMode::Talk,
            arm: EvalArm::Control,
            decision: dec.clone(),
            receipt: rec,
        },
    )?;

    // Exercise sidecar filters: empty line in calls.jsonl, non-json file, wrong-day tool round,
    // malformed JSON body & non-null `"error"` HTTP body.
    fs::create_dir_all(arm_dir.join("relay/seats"))?;
    fs::write(
        arm_dir.join("relay/calls.jsonl"),
        format!(
            "\n{}\n",
            serde_json::to_string(&CallLogEntry {
                time_utc: "2026-11-23T00:00:00Z".to_owned(),
                day: "02".to_owned(),
                round: Round::Act,
                guild: "Foundry".to_owned(),
                attempt: 1,
                trigger: "turn".to_owned(),
                model_requested: "m".to_owned(),
                model_returned: Some("m".to_owned()),
                request_body_sha256: "00".repeat(32),
                http_reply_sha256: None,
                reply_sha256: None,
                prompt_tokens: Some(10),
                completion_tokens: Some(10),
                total_tokens: Some(20),
                reasoning_tokens: None,
                cost_unit: league_relay::CostUnit::default(),
                wall_secs: Some(0.1),
                cost: None,
                charged_credits: 0.0,
                latency_ms: 100,
                outcome: "TOOL_CALL".to_owned(),
                error: None,
                reservation_id: Some("r_ok".to_owned()),
            })?
        ),
    )?;
    fs::write(arm_dir.join("relay/seats/ignore.txt"), b"skip me")?;
    fs::write(
        arm_dir.join("relay/seats/Foundry.day-02-act.tool-round-wrong.json"),
        serde_json::to_string(&SavedToolRoundRecord {
            day: "01".to_owned(),
            round: Round::Act,
            guild: "Foundry".to_owned(),
            run_id: "r".to_owned(),
            reservation_id: "r_ok".to_owned(),
            request_body_sha256: "00".repeat(32),
            round_index: 0,
            attempt: 1,
            outcome: "TOOL_CALL".to_owned(),
            assistant_content: None,
            assistant_content_sha256: None,
            reasoning: Some("ignored wrong day".to_owned()),
            assistant_reply_sha256: "00".repeat(32),
            body_redacted: false,
            calls: Vec::new(),
        })?,
    )?;
    fs::write(
        arm_dir.join("relay/seats/Foundry.day-02-act.http-reply-round-badbody.json"),
        serde_json::to_string(&SavedHttpReplyRecord {
            day: "02".to_owned(),
            round: Round::Act,
            guild: "Foundry".to_owned(),
            run_id: "r".to_owned(),
            reservation_id: "r_ok".to_owned(),
            request_body_sha256: "00".repeat(32),
            round_index: 0,
            attempt: 1,
            status: 200,
            is_late: false,
            body_redacted: false,
            redacted_secret_field_paths: Vec::new(),
            body_sha256: "00".repeat(32),
            body: "not-json".to_owned(),
            prompt_token_ids_sha256: None,
            prompt_token_ids_count: None,
            prompt_token_ids_source: None,
            prompt_token_ids_template_check: None,
        })?,
    )?;
    fs::write(
        arm_dir.join("relay/seats/Foundry.day-02-act.http-reply-round-errbody.json"),
        serde_json::to_string(&SavedHttpReplyRecord {
            day: "02".to_owned(),
            round: Round::Act,
            guild: "Foundry".to_owned(),
            run_id: "r".to_owned(),
            reservation_id: "r_ok".to_owned(),
            request_body_sha256: "00".repeat(32),
            round_index: 1,
            attempt: 1,
            status: 200,
            is_late: false,
            body_redacted: false,
            redacted_secret_field_paths: Vec::new(),
            body_sha256: "00".repeat(32),
            body: json!({"error": {"message": "upstream err"}}).to_string(),
            prompt_token_ids_sha256: None,
            prompt_token_ids_count: None,
            prompt_token_ids_source: None,
            prompt_token_ids_template_check: None,
        })?,
    )?;
    let counts = measure_reasoning_length_in_memory(&arm_dir, 2, "Foundry")?;
    assert_eq!(counts.tool_round_reasoning_chars, None);
    assert_eq!(counts.http_reply_reasoning_chars, None);
    assert_eq!(counts.total_reasoning_chars, None);
    assert_eq!(counts.unparseable_sidecars, 1);

    let s0 = sample_submission(0);
    let c0 = candidate_id_for_submission(&s0)?;
    let blind_dir = tmp.path().join("blind");
    let arm_map_dir = tmp.path().join("arm_map");
    let scores_dir = tmp.path().join("scores");
    let cfg = ScorerConfig::default();
    build_blind_and_score_files(
        &blind_dir,
        &arm_map_dir,
        &scores_dir,
        "g_m5_neg-day-02-seat-0",
        "g_m5_neg",
        2,
        0,
        &save_sha,
        &seal_sha,
        &pair_sha,
        &cfg,
        vec![ArmMapEntry {
            arm: EvalArm::Control,
            replicate: 0,
            committed_candidate_id: c0,
            first_draft_candidate_id: None,
            decision_path: dec_p,
            outcome_path: out_p,
            setup_path: Some(src_dir.join("setup.json")),
            manifest_path: Some(src_dir.join("manifest.json")),
        }],
        vec![(s0, 10.0, Some(0.5))],
        false,
    )?;

    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;
    let vc_path = verify_source_ckpts(&src_dir, &tmp.path().join("vc_merge.json"))?;
    let freeze_path = tmp.path().join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir,
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &clean_git_sha_source(),
    )?;

    let out_dir = tmp.path().join("joined");
    run_join(&JoinOptions {
        freeze_path,
        arm_map_dir,
        scores_dir,
        out_dir: out_dir.clone(),
        verified_checkpoints_path: Some(vc_path),
        blind_dir: None,
        source_game_dirs: vec![parent_src],
    })?;
    let joined: JoinedDecisionsFile =
        serde_json::from_slice(&fs::read(out_dir.join("joined_decisions.json"))?)?;
    let m = &joined.rows[0].mechanisms;
    assert_eq!(m.extracted_outgoing_commitments, Some(2));
    assert_eq!(m.applicable_commitments, Some(2));
    assert_eq!(m.honoured_commitments, Some(0));
    assert_eq!(m.third_party_fill_commitments, Some(1));
    assert_eq!(m.honour_rate, Some(0.0));
    assert_eq!(m.unparseable_sidecars, 1);
    Ok(())
}

struct SingleJoinHarness {
    src_dir: PathBuf,
    arm_dir: PathBuf,
    dec_path: PathBuf,
    out_path: PathBuf,
    freeze_path: PathBuf,
    vc_path: PathBuf,
    arm_map_dir: PathBuf,
    scores_dir: PathBuf,
}

fn prepare_single_decision_join(
    base: &Path,
    host: &mut EvalHost,
    game_id: &str,
) -> Result<SingleJoinHarness, Box<dyn std::error::Error>> {
    let src_dir = base.join("src");
    let (save_sha, seal_sha, pair_sha) =
        write_minimal_source_game(&src_dir, host, EvalArm::Control, game_id)?;

    let arm_dir = base.join("arm_ctrl");
    let dec = base_decision_record(EvalArm::Control, 1, 0, "Foundry");
    let (dec_path, out_path) = write_arm_outcome_pair(
        &arm_dir,
        &dec,
        &BranchArmOutcome {
            source_game_id: game_id.to_owned(),
            table_id: 9200,
            day: 1,
            focal_seat: 0,
            focal_guild: "Foundry".to_owned(),
            communication: CommunicationMode::Talk,
            arm: EvalArm::Control,
            decision: dec.clone(),
            receipt: default_receipt(1),
        },
    )?;

    let blind_dir = base.join("blind");
    let arm_map_dir = base.join("arm_map");
    let scores_dir = base.join("scores");
    let cfg = ScorerConfig::default();
    let s0 = sample_submission(0);
    let c0 = candidate_id_for_submission(&s0)?;
    build_blind_and_score_files(
        &blind_dir,
        &arm_map_dir,
        &scores_dir,
        &format!("{game_id}-day-01-seat-0"),
        game_id,
        1,
        0,
        &save_sha,
        &seal_sha,
        &pair_sha,
        &cfg,
        vec![ArmMapEntry {
            arm: EvalArm::Control,
            replicate: 0,
            committed_candidate_id: c0,
            first_draft_candidate_id: None,
            decision_path: dec_path.clone(),
            outcome_path: out_path.clone(),
            setup_path: Some(src_dir.join("setup.json")),
            manifest_path: Some(src_dir.join("manifest.json")),
        }],
        vec![(s0, 10.0, Some(0.5))],
        false,
    )?;

    let cfg_path = base.join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;
    let vc_path = verify_source_ckpts(&src_dir, &base.join("vc.json"))?;
    let freeze_path = base.join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir,
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            verified_checkpoints_path: vc_path.clone(),
            scorer_crate_dir: None,
        },
        &clean_git_sha_source(),
    )?;

    Ok(SingleJoinHarness {
        src_dir,
        arm_dir,
        dec_path,
        out_path,
        freeze_path,
        vc_path,
        arm_map_dir,
        scores_dir,
    })
}

fn run_single_join(
    harness: &SingleJoinHarness,
    out_dir: &Path,
) -> Result<(JoinedDecisionsFile, PerArmSummaryFile), Box<dyn std::error::Error>> {
    run_join(&JoinOptions {
        freeze_path: harness.freeze_path.clone(),
        arm_map_dir: harness.arm_map_dir.clone(),
        scores_dir: harness.scores_dir.clone(),
        out_dir: out_dir.to_path_buf(),
        verified_checkpoints_path: Some(harness.vc_path.clone()),
        blind_dir: None,
        source_game_dirs: vec![harness.src_dir.clone()],
    })?;
    let joined: JoinedDecisionsFile =
        serde_json::from_slice(&fs::read(out_dir.join("joined_decisions.json"))?)?;
    let summary: PerArmSummaryFile =
        serde_json::from_slice(&fs::read(out_dir.join("per_arm_summary.json"))?)?;
    Ok((joined, summary))
}

#[test]
fn unparseable_optional_sidecars_emit_null_count_and_preserve_s3a_report() -> TestResult {
    let tmp = TmpDir::new("unparseable_sidecars")?;

    let write_clean_talk_and_neg = |src_dir: &Path| -> TestResult {
        fs::write(
            src_dir.join("to-seat/day-01-act-Quarry.negotiation.json"),
            to_sorted_pretty_json(&NegotiationInputs {
                batna: "b".to_owned(),
                whispers: vec![UntrustedWhisper {
                    from_guild: "Foundry".to_owned(),
                    intent_header: None,
                    body: "I will sell 1 raw 0 at 10.".to_owned(),
                }],
            })?,
        )?;
        fs::write(
            src_dir.join("decisions/day-01-talk-Foundry.json"),
            to_sorted_pretty_json(&EvalDecisionRecord {
                raw_reply: Some(
                    json!({
                        "messages": [{ "to_guild": "Quarry", "body": "I will sell 1 raw 0 at 10." }],
                        "market_actions": [],
                        "bundle": {},
                        "notes": "talk"
                    })
                    .to_string(),
                ),
                ..base_decision_record(EvalArm::Control, 1, 0, "Foundry")
            })?,
        )?;
        Ok(())
    };

    let write_clean_relay = |arm_dir: &Path| -> TestResult {
        fs::create_dir_all(arm_dir.join("relay/seats"))?;
        fs::write(
            arm_dir.join("relay/calls.jsonl"),
            format!(
                "{}\n{}\n",
                serde_json::to_string(&make_call_log_entry(
                    "01",
                    Round::Act,
                    "Foundry",
                    "TOOL_CALL",
                    "r0"
                ))?,
                serde_json::to_string(&make_call_log_entry(
                    "01",
                    Round::Act,
                    "Foundry",
                    "OK",
                    "r1"
                ))?,
            ),
        )?;
        fs::write(
            arm_dir.join("relay/seats/Foundry.day-01-act.tool-round-0.json"),
            serde_json::to_string(&make_tool_round_record(
                "01",
                "Foundry",
                "r0",
                0,
                Some("tool"),
            ))?,
        )?;
        fs::write(
            arm_dir.join("relay/seats/Foundry.day-01-act.http-reply-round-1.json"),
            serde_json::to_string(&make_http_reply_record(
                "01",
                "Foundry",
                "r1",
                1,
                200,
                json!({"choices": [{"message": {"reasoning": "http_reply", "content": "{}"}}]})
                    .to_string(),
            ))?,
        )?;
        Ok(())
    };

    // 1) Corrupt tool-round JSON -> tool & total null, http intact, S3a intact.
    {
        let mut host = EvalHost::from_standard_setup(TableId(9210), 4, 1, 11)?;
        let h = prepare_single_decision_join(&tmp.path().join("c_tool"), &mut host, "g_t")?;
        write_clean_relay(&h.arm_dir)?;
        write_clean_talk_and_neg(&h.src_dir)?;
        fs::write(
            h.arm_dir
                .join("relay/seats/Foundry.day-01-act.tool-round-0.json"),
            b"{truncated",
        )?;
        let (joined, sum) = run_single_join(&h, &tmp.path().join("c_tool/out"))?;
        assert_eq!(joined.rows[0].committed_ev_total_equity, 10.0);
        assert_eq!(joined.rows[0].committed_score_norm, Some(0.5));
        assert_eq!(sum.arms[0].mean_ev_total_equity, Some(10.0));
        let m = &joined.rows[0].mechanisms;
        assert_eq!(m.tool_round_reasoning_chars, None);
        assert_eq!(m.http_reply_reasoning_chars, Some(10));
        assert_eq!(m.total_reasoning_chars, None);
        assert_eq!(m.unparseable_sidecars, 1);
        assert_eq!(sum.arms[0].mechanisms.n_unparseable_sidecars, 1);
        assert_eq!(m.extracted_outgoing_commitments, Some(1));
    }

    // 2) Corrupt HTTP-reply JSON (and non-JSON HTTP 200 body) -> http & total null, tool intact, S3a intact.
    for (label, payload) in [
        ("c_http_env", b"{bad-http".to_vec()),
        (
            "c_http_body",
            serde_json::to_vec(&make_http_reply_record(
                "01",
                "Foundry",
                "r1",
                1,
                200,
                "plain non-json body".to_owned(),
            ))?,
        ),
    ] {
        let mut host = EvalHost::from_standard_setup(TableId(9211), 4, 1, 12)?;
        let h = prepare_single_decision_join(&tmp.path().join(label), &mut host, "g_h")?;
        write_clean_relay(&h.arm_dir)?;
        fs::write(
            h.arm_dir
                .join("relay/seats/Foundry.day-01-act.http-reply-round-1.json"),
            payload,
        )?;
        let (joined, sum) = run_single_join(&h, &tmp.path().join(label).join("out"))?;
        assert_eq!(joined.rows[0].committed_ev_total_equity, 10.0);
        let m = &joined.rows[0].mechanisms;
        assert_eq!(m.tool_round_reasoning_chars, Some(4));
        assert_eq!(m.http_reply_reasoning_chars, None);
        assert_eq!(m.total_reasoning_chars, None);
        assert_eq!(m.unparseable_sidecars, 1);
        assert_eq!(sum.arms[0].mechanisms.n_unparseable_sidecars, 1);
    }

    // 3) Corrupt line in calls.jsonl -> all 3 reasoning char metrics null, S3a intact.
    {
        let mut host = EvalHost::from_standard_setup(TableId(9212), 4, 1, 13)?;
        let h = prepare_single_decision_join(&tmp.path().join("c_calls"), &mut host, "g_c")?;
        write_clean_relay(&h.arm_dir)?;
        fs::write(h.arm_dir.join("relay/calls.jsonl"), "{bad-call-line}\n")?;
        let (joined, sum) = run_single_join(&h, &tmp.path().join("c_calls/out"))?;
        assert_eq!(joined.rows[0].committed_ev_total_equity, 10.0);
        let m = &joined.rows[0].mechanisms;
        assert_eq!(m.tool_round_reasoning_chars, None);
        assert_eq!(m.http_reply_reasoning_chars, None);
        assert_eq!(m.total_reasoning_chars, None);
        assert_eq!(m.unparseable_sidecars, 1);
        assert_eq!(sum.arms[0].mechanisms.n_unparseable_sidecars, 1);
    }

    // 4) Corrupt negotiation.json & 5) corrupt talk decision -> Measure 5(b) all None, 5(a) + S3a preserved.
    for (label, rel_path) in [
        ("c_neg", "to-seat/day-01-act-Quarry.negotiation.json"),
        ("c_talk", "decisions/day-01-talk-Foundry.json"),
    ] {
        let mut host = EvalHost::from_standard_setup(TableId(9213), 4, 1, 14)?;
        let h = prepare_single_decision_join(&tmp.path().join(label), &mut host, "g_5b")?;
        write_clean_talk_and_neg(&h.src_dir)?;
        fs::write(h.src_dir.join(rel_path), b"{corrupt")?;
        let (joined, sum) = run_single_join(&h, &tmp.path().join(label).join("out"))?;
        assert_eq!(joined.rows[0].committed_ev_total_equity, 10.0);
        assert_eq!(joined.rows[0].committed_score_norm, Some(0.5));
        let m = &joined.rows[0].mechanisms;
        assert_eq!(m.pre_act_pledges_count, Some(0));
        assert_eq!(m.extracted_outgoing_commitments, None);
        assert_eq!(m.applicable_commitments, None);
        assert_eq!(m.honoured_commitments, None);
        assert_eq!(m.honour_rate, None);
        assert_eq!(m.focal_talk_counts, None);
        assert_eq!(m.unparseable_sidecars, 1);
        assert_eq!(sum.arms[0].mechanisms.n_unparseable_sidecars, 1);
    }

    // 6) Corrupt scoreboard.json -> terminal_broken_promises_count == None, 5(a) pre-act + 5(b) + S3a preserved,
    // and corrupt required decision.json still fails closed.
    {
        let mut host = EvalHost::from_standard_setup(TableId(9214), 4, 1, 15)?;
        let h = prepare_single_decision_join(&tmp.path().join("c_sb"), &mut host, "g_sb")?;
        write_clean_talk_and_neg(&h.src_dir)?;
        fs::write(h.src_dir.join("scoreboard.json"), b"{bad-scoreboard")?;
        let (joined, sum) = run_single_join(&h, &tmp.path().join("c_sb/out"))?;
        assert_eq!(joined.rows[0].committed_ev_total_equity, 10.0);
        let m = &joined.rows[0].mechanisms;
        assert_eq!(m.terminal_broken_promises_count, None);
        assert_eq!(m.pre_act_pledges_count, Some(0));
        assert_eq!(m.extracted_outgoing_commitments, Some(1));
        assert_eq!(m.unparseable_sidecars, 1);
        assert_eq!(sum.arms[0].mechanisms.n_unparseable_sidecars, 1);

        fs::write(&h.dec_path, b"{corrupt-required-decision")?;
        assert!(run_single_join(&h, &tmp.path().join("c_sb/out_fail")).is_err());
        let clean_dec = fs::read_to_string(&h.out_path)?;
        let outcome: BranchArmOutcome = serde_json::from_str(&clean_dec)?;
        fs::write(&h.dec_path, to_sorted_pretty_json(&outcome.decision)?)?;
        fs::write(&h.out_path, b"{corrupt-required-outcome")?;
        assert!(run_single_join(&h, &tmp.path().join("c_sb/out_fail2")).is_err());
    }
    Ok(())
}

#[test]
fn reasoning_null_vs_zero_and_missing_calls_jsonl() -> TestResult {
    let tmp = TmpDir::new("reasoning_null_zero")?;

    // 1) Missing calls.jsonl with valid populated sidecars -> None (no unfiltered counting), unparseable == 0.
    let arm_no_calls = tmp.path().join("no_calls");
    fs::create_dir_all(arm_no_calls.join("relay/seats"))?;
    fs::write(
        arm_no_calls.join("relay/seats/Foundry.day-01-act.tool-round-0.json"),
        serde_json::to_string(&make_tool_round_record(
            "01",
            "Foundry",
            "r0",
            0,
            Some("uncounted without calls"),
        ))?,
    )?;
    fs::write(
        arm_no_calls.join("relay/seats/Foundry.day-01-act.http-reply-round-1.json"),
        serde_json::to_string(&make_http_reply_record(
            "01",
            "Foundry",
            "r1",
            1,
            200,
            json!({"choices": [{"message": {"reasoning": "also uncounted"}}]}).to_string(),
        ))?,
    )?;
    let no_calls_counts = measure_reasoning_length_in_memory(&arm_no_calls, 1, "Foundry")?;
    assert_eq!(no_calls_counts.tool_round_reasoning_chars, None);
    assert_eq!(no_calls_counts.http_reply_reasoning_chars, None);
    assert_eq!(no_calls_counts.total_reasoning_chars, None);
    assert_eq!(no_calls_counts.unparseable_sidecars, 0);

    // 2) Excluded / omitted reasoning (`reasoning: None` / missing key) with clean calls.jsonl -> None, unparseable == 0.
    let arm_excluded = tmp.path().join("excluded");
    fs::create_dir_all(arm_excluded.join("relay/seats"))?;
    fs::write(
        arm_excluded.join("relay/calls.jsonl"),
        format!(
            "{}\n{}\n",
            serde_json::to_string(&make_call_log_entry(
                "01",
                Round::Act,
                "Foundry",
                "TOOL_CALL",
                "r0"
            ))?,
            serde_json::to_string(&make_call_log_entry(
                "01",
                Round::Act,
                "Foundry",
                "OK",
                "r1"
            ))?,
        ),
    )?;
    fs::write(
        arm_excluded.join("relay/seats/Foundry.day-01-act.tool-round-0.json"),
        serde_json::to_string(&make_tool_round_record("01", "Foundry", "r0", 0, None))?,
    )?;
    fs::write(
        arm_excluded.join("relay/seats/Foundry.day-01-act.http-reply-round-1.json"),
        serde_json::to_string(&make_http_reply_record(
            "01",
            "Foundry",
            "r1",
            1,
            200,
            json!({"choices": [{"message": {"content": "{}"}}]}).to_string(),
        ))?,
    )?;
    let excluded_counts = measure_reasoning_length_in_memory(&arm_excluded, 1, "Foundry")?;
    assert_eq!(excluded_counts.tool_round_reasoning_chars, None);
    assert_eq!(excluded_counts.http_reply_reasoning_chars, None);
    assert_eq!(excluded_counts.total_reasoning_chars, None);
    assert_eq!(excluded_counts.unparseable_sidecars, 0);

    // 3) Clean sidecars holding explicit empty strings (`""`) -> Some(0), summed cleanly in join summary.
    let mut host = EvalHost::from_standard_setup(TableId(9215), 4, 1, 16)?;
    let h = prepare_single_decision_join(&tmp.path().join("empty_ok"), &mut host, "g_emp")?;
    fs::create_dir_all(h.arm_dir.join("relay/seats"))?;
    fs::write(
        h.arm_dir.join("relay/calls.jsonl"),
        format!(
            "{}\n{}\n",
            serde_json::to_string(&make_call_log_entry(
                "01",
                Round::Act,
                "Foundry",
                "TOOL_CALL",
                "r0"
            ))?,
            serde_json::to_string(&make_call_log_entry(
                "01",
                Round::Act,
                "Foundry",
                "OK",
                "r1"
            ))?,
        ),
    )?;
    fs::write(
        h.arm_dir
            .join("relay/seats/Foundry.day-01-act.tool-round-0.json"),
        serde_json::to_string(&make_tool_round_record("01", "Foundry", "r0", 0, Some("")))?,
    )?;
    fs::write(
        h.arm_dir
            .join("relay/seats/Foundry.day-01-act.http-reply-round-1.json"),
        serde_json::to_string(&make_http_reply_record(
            "01",
            "Foundry",
            "r1",
            1,
            200,
            json!({"choices": [{"message": {"reasoning": "", "content": "{}"}}]}).to_string(),
        ))?,
    )?;
    let (joined, sum) = run_single_join(&h, &tmp.path().join("empty_ok/out"))?;
    let m = &joined.rows[0].mechanisms;
    assert_eq!(m.tool_round_reasoning_chars, Some(0));
    assert_eq!(m.http_reply_reasoning_chars, Some(0));
    assert_eq!(m.total_reasoning_chars, Some(0));
    assert_eq!(m.unparseable_sidecars, 0);
    assert_eq!(sum.arms[0].mechanisms.n_with_reasoning_sidecars, 1);
    assert_eq!(sum.arms[0].mechanisms.mean_total_reasoning_chars, Some(0.0));
    assert_eq!(sum.arms[0].mechanisms.n_unparseable_sidecars, 0);
    Ok(())
}

fn verify_sec72_method_alignment(doc_text: &str, mechanisms_src: &str) -> Result<(), String> {
    let required_doc = [
        "EvalHost::observe_seat",
        "EngineState::{pledges, audited_pledges}",
        "EvalSaveSnapshot::{pledges, audited_pledges}",
        "EvalHost::apply_market_action",
        "TurnEngine::handle_post_offer",
        "preview_seat_bundle",
        "n_unparseable_sidecars",
    ];
    for pat in required_doc {
        if !doc_text.contains(pat) {
            return Err(format!("doc missing `{pat}`"));
        }
    }
    let required_code = ["observe_seat(", "apply_market_action("];
    for pat in required_code {
        if !mechanisms_src.contains(pat) {
            return Err(format!("mechanisms.rs missing `{pat}`"));
        }
    }
    Ok(())
}

#[test]
fn sec72_measure5_methods_align_with_doc_and_preview_equivalence() -> TestResult {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let doc_text = fs::read_to_string(root.join("docs/decision-scorer.md"))?;
    let mech_src = fs::read_to_string(root.join("crates/league-eval/src/mechanisms.rs"))?;
    verify_sec72_method_alignment(&doc_text, &mech_src).map_err(std::io::Error::other)?;
    let drifted_doc = doc_text.replace("EvalHost::apply_market_action", "REMOVED_METHOD");
    assert!(verify_sec72_method_alignment(&drifted_doc, &mech_src).is_err());

    // Build a pre-act checkpoint containing a recorded IntentPledge on Day 2 and check both
    // Measure 5(a) snapshot vs observe_seat parity and Measure 5(b) apply_market_action vs
    // preview_seat_bundle parity for a legal and an illegal offer.
    let tmp = TmpDir::new("m5_equiv")?;
    let mut host = EvalHost::from_standard_setup(TableId(9216), 4, 2, 77)?;
    let mut t1_subs = BTreeMap::new();
    t1_subs.insert(
        0,
        EvalActSubmission {
            market_actions: vec![MarketAction::RecordIntentPledge {
                seat: 0,
                pledge: league_seat_types::IntentPledge {
                    pledge_id: 1,
                    promisor: 0,
                    counterparty: 1,
                    target: league_seat_types::IntentTarget::VoteForSeat {
                        office_turn: 4,
                        candidate: 1,
                    },
                    honor_weight: 50,
                    memo_digest: [9; 32],
                },
            }],
            ..EvalActSubmission::default()
        },
    );
    let _ = host.apply_act_submissions(&t1_subs)?;
    let _ = host.sweep_turn()?;
    host.set_engine_commit(clean_build_sha())?;
    let (save_path, seal_path) = host.write_pre_act_checkpoint(tmp.path())?;
    let snap_val: serde_json::Value = serde_json::from_slice(&fs::read(&save_path)?)?;
    let obs = host.observe_seat(Seat(0))?;
    assert!(
        snap_val["pledges"]
            .as_object()
            .is_some_and(|m| !m.is_empty())
    );
    assert_eq!(snap_val["pledges"], serde_json::to_value(&obs.pledges)?);
    assert_eq!(
        snap_val["audited_pledges"],
        serde_json::to_value(&obs.audited_pledges)?
    );

    let preview_state = obs.to_public_preview_state();
    let legal = MarketAction::PostOffer {
        seat: 0,
        order: OfferProposal {
            side: OfferSide::Sell,
            good: AssetId::Raw(0),
            amount: UNIT_SCALE,
            price_per_unit_cred: 10 * UNIT_SCALE,
            domain_id: None,
            is_hybrid: false,
        },
    };
    let excessive_units = (obs.seats[0].raw[0] / UNIT_SCALE) + 10_000;
    let illegal = MarketAction::PostOffer {
        seat: 0,
        order: OfferProposal {
            side: OfferSide::Sell,
            good: AssetId::Raw(0),
            amount: excessive_units * UNIT_SCALE,
            price_per_unit_cred: 10 * UNIT_SCALE,
            domain_id: None,
            is_hybrid: false,
        },
    };

    for (amount, action, expected_legal) in [
        (UNIT_SCALE, legal, true),
        (excessive_units * UNIT_SCALE, illegal, false),
    ] {
        let mut fork = EvalHost::fork_from_checkpoint(&save_path, &seal_path)?;
        let via_apply = fork.apply_market_action(action).is_ok();
        let preview_args = serde_json::json!({
            "market_actions": [{
                "action_kind": "post_offer",
                "side": "sell",
                "good": { "asset_kind": "raw", "good_index": 0 },
                "amount": amount,
                "price_per_unit_cred": 10 * UNIT_SCALE,
                "domain_id": serde_json::Value::Null,
                "is_hybrid": false,
            }],
            "bundle": {},
        })
        .to_string();
        let wire_raw = execute_preview_tool_call(&preview_state, &preview_args);
        let wire_val: serde_json::Value = serde_json::from_str(&wire_raw)?;
        assert_eq!(
            wire_val["status"], "ok",
            "unexpected wire reply: {wire_raw}"
        );
        let via_preview = wire_val["refused_market_actions"]
            .as_array()
            .is_some_and(Vec::is_empty);
        assert_eq!(via_apply, expected_legal);
        assert_eq!(via_apply, via_preview);
    }
    Ok(())
}

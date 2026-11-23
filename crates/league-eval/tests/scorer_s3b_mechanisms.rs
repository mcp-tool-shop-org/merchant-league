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
    ArmMapEntry, ArmMapStateFile, BranchArmOutcome, CRATE_GIT_SHA, CrateGitShaSource,
    DESCRIPTIVE_ONLY_INFERENTIAL_STATUS, EvalArm, EvalDecisionRecord, EvalManifest,
    EvalManifestSeat, FreezeOptions, HTTP_REPLY_OK_STATUS, JoinOptions, JoinedDecisionsFile,
    MECHANISM_DATA_LIMITS, PASS_B_LIMIT_STATEMENT, PerArmSummaryFile,
    REFUSED_BUNDLE_LEGS_MIN_COUNT, RefusalCodeCounts, RefusedBundleRecord,
    TALK_COMMITMENT_HONOUR_WINDOW_DAYS, TALK_COMMITMENT_MIN_FILL_BPS,
    TALK_COMMITMENT_PRICE_TOLERANCE_MILLI_CRED, TOOL_CALL_USED_MIN_COUNT, WrongPartCounter,
    execute_cli_with_git_sha, measure_reasoning_length_in_memory, run_freeze, run_join,
    summarize_arm_mechanisms,
};
use league_relay::{
    CONTROL_WORDING_ARM, CallLogEntry, NegotiationInputs, Round, SavedHttpReplyRecord,
    SavedToolRoundRecord, UntrustedWhisper,
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

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
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
        arm: EvalArm::On,
        decision: dec_on.clone(),
        receipt: default_receipt(1),
    };
    let (dec_on_p, out_on_p) =
        write_arm_outcome_pair(&tmp.path().join("arm_on"), &dec_on, &out_on)?;

    // Flat state (State 2) on control arm with empty relay/seats dir -> revision_gain_score_norm == None, sidecar chars == Some(0).
    let arm_ctrl_2 = tmp.path().join("arm_ctrl_2");
    let mut dec_ctrl_2 = base_decision_record(EvalArm::Control, 1, 0, "Foundry");
    dec_ctrl_2.completion_tokens = 10;
    let out_ctrl_2 = BranchArmOutcome {
        source_game_id: "g_m14".to_owned(),
        table_id: 9200,
        day: 1,
        focal_seat: 0,
        focal_guild: "Foundry".to_owned(),
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
    let freeze_path = tmp.path().join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
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
    assert_eq!(c2_row.mechanisms.total_reasoning_chars, Some(0));

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
    assert_eq!(ctrl_sum.mechanisms.n_with_reasoning_sidecars, 2);
    assert_eq!(ctrl_sum.mechanisms.n_with_reply_token_ids, 1);
    assert_eq!(ctrl_sum.mechanisms.mean_total_reasoning_chars, Some(5.0));
    assert_eq!(ctrl_sum.mechanisms.mean_reply_token_ids_len, Some(5.0));
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
    let freeze_path = tmp.path().join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
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

#[test]
fn sentinels_never_leak_to_files_stdout_or_stderr_on_success_or_error_paths() -> TestResult {
    let tmp = TmpDir::new("sentinel_noleak")?;
    let mut host = EvalHost::from_standard_setup(TableId(9202), 4, 1, 33)?;
    let src_dir = tmp.path().join("src_sec");
    let (save_sha, seal_sha, pair_sha) =
        write_minimal_source_game(&src_dir, &mut host, EvalArm::Control, "g_sent")?;

    let sentinel = "PRIVATE_REASONING_WHISPER_SENTINEL_7X9Q";
    fs::write(
        src_dir.join("decisions/day-01-talk-Foundry.json"),
        to_sorted_pretty_json(&EvalDecisionRecord {
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
        })?,
    )?;
    fs::write(
        src_dir.join("to-seat/day-01-act-Quarry.negotiation.json"),
        to_sorted_pretty_json(&NegotiationInputs {
            batna: "b".to_owned(),
            whispers: vec![UntrustedWhisper {
                from_guild: "Foundry".to_owned(),
                intent_header: None,
                body: format!("Ambiguous {sentinel}?"),
            }],
        })?,
    )?;

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
            arm: EvalArm::Control,
            decision: dec.clone(),
            receipt: default_receipt(1),
        },
    )?;
    fs::create_dir_all(arm_dir.join("relay/seats"))?;
    let tool_path = arm_dir.join("relay/seats/Foundry.day-01-act.tool-round-0.json");
    fs::write(
        &tool_path,
        serde_json::to_string(&SavedToolRoundRecord {
            day: "01".to_owned(),
            round: Round::Act,
            guild: "Foundry".to_owned(),
            run_id: "r".to_owned(),
            reservation_id: "r0".to_owned(),
            request_body_sha256: "00".repeat(32),
            round_index: 0,
            attempt: 1,
            outcome: "TOOL_CALL".to_owned(),
            assistant_content: None,
            assistant_content_sha256: None,
            reasoning: Some(format!("tool-{sentinel}")),
            assistant_reply_sha256: "00".repeat(32),
            body_redacted: false,
            calls: Vec::new(),
        })?,
    )?;
    let http_path = arm_dir.join("relay/seats/Foundry.day-01-act.http-reply-round-1.json");
    fs::write(
        &http_path,
        serde_json::to_string(&SavedHttpReplyRecord {
            day: "01".to_owned(),
            round: Round::Act,
            guild: "Foundry".to_owned(),
            run_id: "r".to_owned(),
            reservation_id: "r1".to_owned(),
            request_body_sha256: "00".repeat(32),
            round_index: 1,
            attempt: 1,
            status: 200,
            is_late: false,
            body_redacted: false,
            redacted_secret_field_paths: Vec::new(),
            body_sha256: "00".repeat(32),
            body: json!({"choices": [{"message": {"reasoning": format!("http-{sentinel}"), "content": "{}"}}]}).to_string(),
            prompt_token_ids_sha256: None,
            prompt_token_ids_count: None,
            prompt_token_ids_source: None,
            prompt_token_ids_template_check: None,
        })?,
    )?;

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
    let freeze_path = tmp.path().join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
            scorer_crate_dir: None,
        },
        &clean_git_sha_source(),
    )?;

    // 1) Full CLI join success path.
    let out_ok = tmp.path().join("joined_ok");
    let git_sha = clean_git_sha_source();
    let (code, stdout, stderr) = run_eval_cli(
        &[
            "join",
            "--freeze",
            freeze_path.to_str().ok_or("missing expected item")?,
            "--arm-map",
            arm_map_dir.to_str().ok_or("missing expected item")?,
            "--scores",
            scores_dir.to_str().ok_or("missing expected item")?,
            "--out",
            out_ok.to_str().ok_or("missing expected item")?,
            "--source-game",
            src_dir.to_str().ok_or("missing expected item")?,
        ],
        &git_sha,
    );
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(!stdout.contains(sentinel));
    assert!(!stderr.contains(sentinel));
    for entry in fs::read_dir(&out_ok)? {
        let p = entry?.path();
        if p.is_file() {
            let txt = fs::read_to_string(&p)?;
            assert!(
                !txt.contains(sentinel),
                "sentinel leaked into {}",
                p.display()
            );
        }
    }

    // 2) Error path with sentinel inside malformed HTTP sidecar JSON.
    let orig_http = fs::read_to_string(&http_path)?;
    fs::write(
        &http_path,
        json!({"day": 123, "reasoning": sentinel}).to_string(),
    )?;
    let out_err1 = tmp.path().join("joined_err1");
    let (c_err1, out1, err1) = run_eval_cli(
        &[
            "--error-format",
            "json",
            "join",
            "--freeze",
            freeze_path.to_str().ok_or("missing expected item")?,
            "--arm-map",
            arm_map_dir.to_str().ok_or("missing expected item")?,
            "--scores",
            scores_dir.to_str().ok_or("missing expected item")?,
            "--out",
            out_err1.to_str().ok_or("missing expected item")?,
        ],
        &git_sha,
    );
    assert_ne!(c_err1, 0);
    assert!(!out1.contains(sentinel));
    assert!(!err1.contains(sentinel), "stderr leaked sentinel: {err1}");
    fs::write(&http_path, orig_http)?;

    // 3) Error path with sentinel inside malformed negotiation.json.
    fs::write(
        src_dir.join("to-seat/day-01-act-Quarry.negotiation.json"),
        json!({"batna": 1, "whispers": [{"from_guild": sentinel, "body": sentinel}]}).to_string(),
    )?;
    let out_err2 = tmp.path().join("joined_err2");
    let (c_err2, out2, err2) = run_eval_cli(
        &[
            "join",
            "--freeze",
            freeze_path.to_str().ok_or("missing expected item")?,
            "--arm-map",
            arm_map_dir.to_str().ok_or("missing expected item")?,
            "--scores",
            scores_dir.to_str().ok_or("missing expected item")?,
            "--out",
            out_err2.to_str().ok_or("missing expected item")?,
            "--source-game",
            src_dir.to_str().ok_or("missing expected item")?,
        ],
        &git_sha,
    );
    assert_ne!(c_err2, 0);
    assert!(!out2.contains(sentinel));
    assert!(!err2.contains(sentinel), "stderr leaked sentinel: {err2}");

    // 4) Error path with unknown --source-game dir and unknown CLI flag.
    let (c_err3, _, _) = run_eval_cli(&["join", "--unknown-join-flag", "x"], &git_sha);
    assert_ne!(c_err3, 0);
    let err_missing_src = run_join(&JoinOptions {
        freeze_path,
        arm_map_dir,
        scores_dir,
        out_dir: tmp.path().join("joined_err3"),
        blind_dir: None,
        source_game_dirs: vec![tmp.path().join("missing_source_game_dir")],
    })
    .err()
    .ok_or("expected missing --source-game error")?;
    assert_eq!(err_missing_src.code(), "MISSING_SOURCE_SETUP");
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
    let freeze_path = tmp.path().join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir: blind_dir.clone(),
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
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
    assert_eq!(counts.tool_round_reasoning_chars, Some(0));
    assert_eq!(counts.http_reply_reasoning_chars, Some(0));
    assert_eq!(counts.total_reasoning_chars, Some(0));

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
    let freeze_path = tmp.path().join("freeze.json");
    run_freeze(
        &FreezeOptions {
            blind_dir,
            config_path: cfg_path,
            control_wording_arm: CONTROL_WORDING_ARM.to_owned(),
            out_path: freeze_path.clone(),
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
    Ok(())
}

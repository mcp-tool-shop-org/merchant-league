//! Integration, contract, and blinding tests for `league-scorer`.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use league_core::eval::host::checkpoint_verify::verify_checkpoint_causal;
use league_core::{
    AiPreviewRequest, AssetId, CharterRole, CheckProposal, ClubBot, CommitmentHash,
    EVAL_SAVE_FORMAT_VERSION, EVAL_SEAL_FORMAT_VERSION, EscrowProposal, EvalActSubmission,
    EvalHost, GradedMandateCard, IntentPledge, IntentTarget, MandateKind, MarketAction,
    OfferProposal, OfferSide, PublicTableSpec, SealedOrderBundle, Seat, TableId,
    ToStructuredError as _, TurnPhase, mul_units_parity, preview_seat_bundle, replay_eval_season,
    town_import_cost_cred,
};
use league_scorer::cli::run_cli;
use league_scorer::{
    BLIND_SCORING_SCHEMA_VERSION, BlindCandidate, BlindScoringRequest,
    COMPOSITE_GREEDY_RULES_DEFAULT, CRATE_GIT_SHA, DROP_TIE_BREAK_BY_CANDIDATE_ID, EXIT_OK,
    EXIT_RUNTIME_ERROR, EXIT_USER_ERROR, ExtractedTalkCommitment, HONOUR_PPM_SCALE, PostureKind,
    PostureWeights, SYNTH_CHARTER_BID_CRED, SYNTH_CLAIM_MATURING_EXACT,
    SYNTH_CONVERTER_INPUT_DEFICIT_COVER, SYNTH_COUNCIL_SELF_NOMINATION,
    SYNTH_DROP_DEFAULT_ACTION_RISK_CRED, SYNTH_DROP_PROJECT_RISK_CRED,
    SYNTH_DROP_UPKEEP_SELL_RISK_MULTIPLIER, SYNTH_MAX_CONVERTER_BATCHES, SYNTH_MAX_PROJECT_BATCHES,
    SYNTH_PROJECT_RANK_BY_EQUITY, SYNTH_QUOTE_CRED_SHARE_BPS, SYNTH_QUOTE_MAX_UNITS,
    SYNTH_QUOTE_MIN_UNITS, SYNTH_SCALE_SEED_PER_AXIS, SYNTH_SCALE_SEED_POOL_MAX,
    ScenarioRolloutOutcome, ScorerConfig, ScorerError, ScorerFreezeBlindView, StateScoreOutput,
    TALK_EXTRACTOR_RULES_DIGEST_V2, TALK_EXTRACTOR_VERSION_V2, TalkExtractionCounts,
    TalkSourceCounts, VERIFIED_CHECKPOINTS_SCHEMA_VERSION, VerifiedCheckpointEntry,
    VerifiedCheckpointsManifest, apply_talk_commitments, build_arm_independent_request,
    candidate_id_for_submission, canonicalize_talk_commitments,
    compute_blind_inputs_manifest_sha256, compute_blind_inputs_manifest_sha256_from_entries,
    compute_pre_act_pair_sha256_hex, compute_scorer_tree_sha256, decide_peer_turn,
    dedupe_candidates, draw_honour_commitment, draw_scenarios, execute_blind_request,
    generate_arm_independent_candidates, is_valid_sha256_hex, p_honour_to_ppm,
    parse_blind_scoring_request_strict, posture_for_scenario, probe_checkpoint_sanitized,
    prune_refused_market_actions, run_single_rollout, run_single_rollout_with_replay_log,
    score_outcomes, sha256_hex, to_sorted_compact_json, to_sorted_pretty_json,
    validate_talk_commitment, verify_checkpoint_pair,
};
use league_seat_types::UNIT_SCALE;

type TestResult = Result<(), Box<dyn Error>>;
const TEST_ENGINE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct UniqueTmpDir {
    path: PathBuf,
}

impl UniqueTmpDir {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let id = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("league_scorer_{label}_{}_{id}", std::process::id()));
        if dir.exists() {
            let _ = fs::remove_dir_all(&dir);
        }
        fs::create_dir_all(&dir)?;
        Ok(Self { path: dir })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for UniqueTmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn must_ok<T, E: std::fmt::Display>(res: Result<T, E>, ctx: &str) -> T {
    match res {
        Ok(v) => v,
        Err(e) => panic!("{ctx}: {e}"),
    }
}

fn check_blinding_manifest(manifest: &str) -> Result<(), String> {
    const ALLOWED: [&str; 5] = [
        "league-core",
        "league-seat-types",
        "serde",
        "serde_json",
        "sha2",
    ];
    let mut section = String::new();
    for raw_line in manifest.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line.to_owned();
            if section.contains("dev-dependencies")
                || section.contains("build-dependencies")
                || section == "[features]"
            {
                return Err(format!(
                    "disallowed section `{section}` in blinded crate Cargo.toml"
                ));
            }
            if let Some(sub) = section
                .strip_prefix("[dependencies.")
                .and_then(|s| s.strip_suffix(']'))
            {
                let dep = sub.split('.').next().unwrap_or("").trim().trim_matches('"');
                if !ALLOWED.contains(&dep) {
                    return Err(format!("disallowed dependency `{dep}` in league-scorer"));
                }
            }
            continue;
        }
        if (section == "[dependencies]" || section.ends_with(".dependencies]"))
            && let Some((key_raw, _)) = line.split_once('=')
        {
            let dep = key_raw.trim().trim_matches('"');
            if !ALLOWED.contains(&dep) {
                return Err(format!("disallowed dependency `{dep}` in league-scorer"));
            }
        }
    }
    Ok(())
}

fn build_candidate(
    submission: EvalActSubmission,
    in_span: bool,
    is_arm_independent: bool,
) -> Result<BlindCandidate, Box<dyn Error>> {
    let candidate_id = candidate_id_for_submission(&submission)?;
    Ok(BlindCandidate {
        candidate_id,
        in_span,
        is_arm_independent,
        submission,
    })
}

fn build_sorted_candidates(mut items: Vec<BlindCandidate>) -> Vec<BlindCandidate> {
    items.sort_by(|a, b| a.candidate_id.cmp(&b.candidate_id));
    items
}

struct TestCheckpointFixture {
    cas_save: PathBuf,
    cas_seal: PathBuf,
    save_sha: String,
    seal_sha: String,
    pair_sha: String,
    save_bytes: Vec<u8>,
}

fn make_real_checkpoint_pair(
    tmp: &Path,
    table_seed: u64,
    focal_seat: u8,
) -> Result<TestCheckpointFixture, Box<dyn Error>> {
    let mut host = EvalHost::from_standard_setup(TableId(table_seed), 4, 1, 0)?
        .with_engine_commit(TEST_ENGINE_COMMIT)?;
    let raw_ckpt = tmp.join("raw_ckpt");
    fs::create_dir_all(&raw_ckpt)?;
    let (raw_save, raw_seal) = host.write_pre_act_checkpoint(&raw_ckpt)?;
    let save_bytes = fs::read(&raw_save)?;
    let seal_bytes = fs::read(&raw_seal)?;
    let save_sha = sha256_hex(&save_bytes);
    let seal_sha = sha256_hex(&seal_bytes);
    let pair_sha = compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes);

    let cas_dir = tmp.join("checkpoints");
    fs::create_dir_all(&cas_dir)?;
    let cas_save = cas_dir.join(format!("{save_sha}.save.json"));
    let cas_seal = cas_dir.join(format!("{seal_sha}.seal"));
    fs::write(&cas_save, &save_bytes)?;
    fs::write(&cas_seal, &seal_bytes)?;
    let _ = host.observe_seat(Seat(focal_seat))?;
    Ok(TestCheckpointFixture {
        cas_save,
        cas_seal,
        save_sha,
        seal_sha,
        pair_sha,
        save_bytes,
    })
}

#[test]
fn blinding_cargo_toml_forbids_unapproved_dependencies() -> TestResult {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)?;
    assert!(
        check_blinding_manifest(&manifest).is_ok(),
        "real Cargo.toml failed blinding check"
    );
    for forbidden in [
        "league-eval",
        "league-relay",
        "rayon",
        "dev-dependencies",
        "build-dependencies",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "Cargo.toml unexpectedly mentions `{forbidden}`"
        );
    }

    let bad_dep = manifest.replace("[dependencies]", "[dependencies]\nrayon = \"1.10\"");
    assert!(check_blinding_manifest(&bad_dep).is_err());
    let bad_subtable = "[package]\nname=\"x\"\n[dependencies.rayon]\nversion = \"1.10\"\n";
    assert!(check_blinding_manifest(bad_subtable).is_err());
    let bad_eval =
        "[package]\nname=\"x\"\n[dependencies]\nleague-eval = { path = \"../league-eval\" }\n";
    assert!(check_blinding_manifest(bad_eval).is_err());
    let bad_dev = format!("{manifest}\n[dev-dependencies]\ntempfile = \"3.27\"\n");
    assert!(check_blinding_manifest(&bad_dev).is_err());
    Ok(())
}

#[test]
fn request_and_config_validation_refuses_every_error_code() -> TestResult {
    let pilot_default = ScorerConfig::default();
    assert_eq!(pilot_default.scenarios_m, 16);
    assert_eq!(pilot_default.k_synth_max, 32);
    assert!(pilot_default.include_pass_b_cleared_club_bot);
    assert_eq!(pilot_default.p_honour, 0.75);
    assert_eq!(pilot_default.posture_weights.standard, 2);
    assert_eq!(pilot_default.posture_weights.active, 1);
    assert_eq!(pilot_default.posture_weights.conservative, 1);
    assert_eq!(pilot_default.posture_schedule.len(), 4);

    assert!(pilot_default.validate().is_ok());

    let mut cfg_bad_weight = ScorerConfig::s1_standard(4);
    cfg_bad_weight.posture_weights.standard = 0;
    cfg_bad_weight.posture_weights.active = 0;
    cfg_bad_weight.posture_weights.conservative = 0;
    assert_eq!(
        cfg_bad_weight.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_empty_sched = ScorerConfig::s1_standard(4);
    cfg_empty_sched.posture_schedule.clear();
    assert_eq!(
        cfg_empty_sched.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_bad_ratio = ScorerConfig::s1_standard(4);
    cfg_bad_ratio.posture_schedule = vec![PostureKind::Standard, PostureKind::Active];
    assert_eq!(
        cfg_bad_ratio.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );
    let cfg_default_bad_sched = ScorerConfig {
        posture_schedule: vec![
            PostureKind::Standard,
            PostureKind::Active,
            PostureKind::Conservative,
        ],
        ..ScorerConfig::default()
    };
    assert_eq!(
        cfg_default_bad_sched.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );
    let mut cfg_scaled_ratio = ScorerConfig::s1_standard(4);
    cfg_scaled_ratio.posture_schedule = vec![
        PostureKind::Standard,
        PostureKind::Standard,
        PostureKind::Active,
    ];
    cfg_scaled_ratio.posture_weights.standard = 4;
    cfg_scaled_ratio.posture_weights.active = 2;
    assert!(cfg_scaled_ratio.validate().is_ok());

    let mut cfg_bad_greedy = ScorerConfig::s1_standard(4);
    cfg_bad_greedy.composite_greedy_rules = "unknown_greedy_rule".to_owned();
    assert_eq!(
        cfg_bad_greedy.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_empty_slots = ScorerConfig::s1_standard(4);
    cfg_empty_slots.synth_slot_fill_order.clear();
    assert_eq!(
        cfg_empty_slots.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_bad_slot = ScorerConfig::s1_standard(4);
    cfg_bad_slot.synth_slot_fill_order = vec!["unknown_bucket".to_owned()];
    assert_eq!(
        cfg_bad_slot.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_dup_slot = ScorerConfig::s1_standard(4);
    cfg_dup_slot.synth_slot_fill_order =
        vec!["claims_council".to_owned(), "claims_council".to_owned()];
    assert_eq!(
        cfg_dup_slot.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_odd_m = ScorerConfig::s1_standard(3);
    assert_eq!(
        cfg_odd_m.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );
    cfg_odd_m = ScorerConfig::s1_standard(0);
    assert_eq!(
        cfg_odd_m.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_bad_mode = ScorerConfig::s1_standard(4);
    cfg_bad_mode.mandate_stratification_mode = "other".to_owned();
    assert_eq!(
        cfg_bad_mode.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_bad_p = ScorerConfig::s1_standard(4);
    cfg_bad_p.p_honour = 1.5;
    assert_eq!(
        cfg_bad_p.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_bad_span = ScorerConfig::s1_standard(4);
    cfg_bad_span.prescreen_min_arm_indep_span = -1;
    assert_eq!(
        cfg_bad_span.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_bad_digest = ScorerConfig::s1_standard(4);
    cfg_bad_digest.talk_extractor_rules_digest = "xyz".to_owned();
    assert_eq!(
        cfg_bad_digest.validate().err().map(|e| e.code()),
        Some("INVALID_HEX_DIGEST")
    );

    let mut cfg_bad_ver = ScorerConfig::s1_standard(4);
    cfg_bad_ver.talk_extractor_version = "  ".to_owned();
    assert_eq!(
        cfg_bad_ver.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let cfg = ScorerConfig::s1_standard(4);
    assert!(cfg.validate().is_ok());
    let cfg_sha = cfg.sha256_hex()?;
    assert!(is_valid_sha256_hex(&cfg_sha));

    // Deny unknown fields on ScorerConfig and BlindScoringRequest.
    let mut cfg_val = serde_json::to_value(&cfg)?;
    if let Some(obj) = cfg_val.as_object_mut() {
        obj.insert("unexpected_extra".to_owned(), serde_json::json!(true));
    }
    assert!(serde_json::from_value::<ScorerConfig>(cfg_val).is_err());

    let c_passive = build_candidate(EvalActSubmission::default(), true, true)?;
    let c_import = build_candidate(
        EvalActSubmission {
            market_actions: vec![MarketAction::TownImport {
                good: AssetId::Raw(0),
                seat: 0,
                amount: 100,
            }],
            pass_b_actions: Vec::new(),
            bundle: SealedOrderBundle::default(),
        },
        true,
        true,
    )?;
    let sorted = build_sorted_candidates(vec![c_passive.clone(), c_import.clone()]);
    let good_hex = sha256_hex(b"good");
    let valid_req = BlindScoringRequest {
        candidates: sorted,
        focal_seat: 0,
        pre_act_pair_sha256: good_hex.clone(),
        pre_act_save_sha256: good_hex.clone(),
        pre_act_seal_sha256: good_hex.clone(),
        schema_version: BLIND_SCORING_SCHEMA_VERSION,
        scorer_config_sha256: cfg_sha.clone(),
        scorer_seed: 99,
        state_id: "game1_day-01_seat-0".to_owned(),
        talk_commitments: Vec::new(),
        talk_extraction_counts: TalkExtractionCounts::default(),
    };
    assert!(valid_req.validate(&cfg).is_ok());

    let mut req_val = serde_json::to_value(&valid_req)?;
    if let Some(obj) = req_val.as_object_mut() {
        obj.insert("arm".to_owned(), serde_json::json!("on"));
    }
    assert!(serde_json::from_value::<BlindScoringRequest>(req_val).is_err());

    let mut bad_ver = valid_req.clone();
    bad_ver.schema_version = 2;
    assert_eq!(
        bad_ver.validate(&cfg).err().map(|e| e.code()),
        Some("UNSUPPORTED_SCHEMA_VERSION")
    );

    for bad_state in ["", "..", "a/b", "a:b", "has space"] {
        let mut r = valid_req.clone();
        r.state_id = bad_state.to_owned();
        assert_eq!(
            r.validate(&cfg).err().map(|e| e.code()),
            Some("INVALID_STATE_ID")
        );
    }

    let mut bad_hex = valid_req.clone();
    bad_hex.pre_act_save_sha256 = good_hex.to_ascii_uppercase();
    assert_eq!(
        bad_hex.validate(&cfg).err().map(|e| e.code()),
        Some("INVALID_HEX_DIGEST")
    );

    let mut bad_seat = valid_req.clone();
    bad_seat.focal_seat = 4;
    assert_eq!(
        bad_seat.validate(&cfg).err().map(|e| e.code()),
        Some("INVALID_FOCAL_SEAT")
    );

    let mut bad_talk = valid_req.clone();
    bad_talk.talk_commitments.push(ExtractedTalkCommitment {
        counterparty_seat: 1,
        offered_action: MarketAction::TownImport {
            good: AssetId::Raw(0),
            seat: 0,
            amount: 50,
        },
        proposer_seat: 0,
    });
    let err_talk = bad_talk
        .validate(&cfg)
        .err()
        .ok_or("expected INVALID_TALK_COMMITMENT")?;
    assert_eq!(err_talk.code(), "INVALID_TALK_COMMITMENT");
    assert_eq!(err_talk.exit_code(), EXIT_USER_ERROR);

    let mut bad_counts = valid_req.clone();
    bad_counts.talk_extraction_counts = TalkExtractionCounts {
        negotiation: TalkSourceCounts {
            ambiguous: 1,
            matched: 1,
            seen: 3,
        },
        talk: TalkSourceCounts::default(),
        total: TalkSourceCounts {
            ambiguous: 1,
            matched: 1,
            seen: 3,
        },
    };
    assert_eq!(
        bad_counts.validate(&cfg).err().map(|e| e.code()),
        Some("INVALID_TALK_COMMITMENT")
    );
    let bad_sum = TalkExtractionCounts::from_sources(
        TalkSourceCounts {
            ambiguous: 1,
            matched: u64::MAX,
            seen: u64::MAX,
        },
        TalkSourceCounts::default(),
    );
    assert_eq!(
        bad_sum.err().map(|e| e.code()),
        Some("INVALID_TALK_COMMITMENT")
    );
    let mut mismatch_total = valid_req.clone();
    mismatch_total.talk_extraction_counts = TalkExtractionCounts {
        negotiation: TalkSourceCounts {
            ambiguous: 1,
            matched: 1,
            seen: 2,
        },
        talk: TalkSourceCounts {
            ambiguous: 0,
            matched: 1,
            seen: 1,
        },
        total: TalkSourceCounts {
            ambiguous: 0,
            matched: 2,
            seen: 2,
        },
    };
    assert_eq!(
        mismatch_total.validate(&cfg).err().map(|e| e.code()),
        Some("INVALID_TALK_COMMITMENT")
    );

    let mut bad_cfg_hash = valid_req.clone();
    bad_cfg_hash.scorer_config_sha256 = good_hex.clone();
    assert_eq!(
        bad_cfg_hash.validate(&cfg).err().map(|e| e.code()),
        Some("CONFIG_HASH_MISMATCH")
    );

    let mut empty_cands = valid_req.clone();
    empty_cands.candidates.clear();
    assert_eq!(
        empty_cands.validate(&cfg).err().map(|e| e.code()),
        Some("EMPTY_CANDIDATES")
    );

    let mut bad_cand_hex = valid_req.clone();
    bad_cand_hex.candidates[0].candidate_id = "short".to_owned();
    assert_eq!(
        bad_cand_hex.validate(&cfg).err().map(|e| e.code()),
        Some("INVALID_HEX_DIGEST")
    );

    let mut unsorted = valid_req.clone();
    unsorted.candidates.reverse();
    assert_eq!(
        unsorted.validate(&cfg).err().map(|e| e.code()),
        Some("UNSORTED_CANDIDATES")
    );

    let mut dup = valid_req.clone();
    dup.candidates[1] = dup.candidates[0].clone();
    assert_eq!(
        dup.validate(&cfg).err().map(|e| e.code()),
        Some("DUPLICATE_CANDIDATE_ID")
    );

    let mut mismatch = valid_req.clone();
    let other_hash = sha256_hex(b"other");
    mismatch.candidates[0].candidate_id = other_hash;
    mismatch
        .candidates
        .sort_by(|a, b| a.candidate_id.cmp(&b.candidate_id));
    assert_eq!(
        mismatch.validate(&cfg).err().map(|e| e.code()),
        Some("CANDIDATE_ID_MISMATCH")
    );

    let mut no_span = valid_req.clone();
    for c in &mut no_span.candidates {
        c.in_span = false;
    }
    assert_eq!(
        no_span.validate(&cfg).err().map(|e| e.code()),
        Some("NO_IN_SPAN_CANDIDATES")
    );

    // Exercise structured error metadata across all remaining error variants.
    let all_errs = vec![
        ScorerError::Usage("bad flag".into()),
        ScorerError::Json("parse fail".into()),
        ScorerError::NoBlindInputs("empty".into()),
        ScorerError::InvalidCheckpointsLayout("bad layout".into()),
        ScorerError::CheckpointNotFound("missing".into()),
        ScorerError::CheckpointHashMismatch("bad sha".into()),
        ScorerError::InvalidCheckpointState("seat count 5".into()),
        ScorerError::FocalMandateNotInDeck,
        ScorerError::InsufficientMandatePool {
            pool_len: 2,
            needed: 3,
        },
        ScorerError::Engine("engine".into()),
        ScorerError::Io("disk".into()),
    ];
    for e in all_errs {
        let txt = e.to_string();
        assert!(txt.starts_with(e.code()));
        let s = e.to_structured_error();
        assert_eq!(s.code, e.code());
        assert!(!e.hint().is_empty());
        assert!(e.exit_code() == EXIT_USER_ERROR || e.exit_code() == EXIT_RUNTIME_ERROR);
    }
    Ok(())
}

#[test]
fn scenario_draw_is_undealt_permutation_excluding_focal_slot_and_seed_sensitive() -> TestResult {
    let host = EvalHost::from_standard_setup(TableId(8101), 4, 1, 0)?;
    let obs = host.observe_seat(Seat(1))?;
    let focal_mandate = *obs
        .own_mandate()
        .ok_or("SeatObservation must include own_mandate")?;
    let pair_sha = sha256_hex(b"pair-8101");

    let focal_card_id = obs
        .spec
        .mandate_deck
        .iter()
        .find(|c| c.kind == focal_mandate.kind && c.bonus_equity == focal_mandate.bonus_equity)
        .map(|c| c.card_id)
        .ok_or("focal mandate entry must appear in spec.mandate_deck")?;

    let expected_undealt_ids: Vec<u16> = obs
        .spec
        .mandate_deck
        .iter()
        .filter(|c| c.card_id != focal_card_id)
        .map(|c| c.card_id)
        .collect();
    assert_eq!(expected_undealt_ids.len(), 7);

    let draws_a = draw_scenarios(&obs.spec, focal_mandate, 1, 777, &pair_sha, 1, 16)?;
    let draws_b = draw_scenarios(&obs.spec, focal_mandate, 1, 777, &pair_sha, 1, 16)?;
    let draws_other_seed = draw_scenarios(&obs.spec, focal_mandate, 1, 778, &pair_sha, 1, 16)?;
    assert_eq!(draws_a, draws_b);
    assert_ne!(draws_a, draws_other_seed);

    for scen in &draws_a {
        assert_eq!(scen.peer_mandates.len(), 3);
        assert!(!scen.peer_mandates.contains_key(&1));
        assert_eq!(scen.selected_card_ids.len(), 3);
        let mut seen_ids = std::collections::BTreeSet::new();
        for &(peer_s, card_id) in &scen.selected_card_ids {
            assert_ne!(peer_s, 1);
            assert_ne!(card_id, focal_card_id);
            assert!(expected_undealt_ids.contains(&card_id));
            assert!(seen_ids.insert(card_id), "duplicate card_id {card_id}");
        }
    }

    // Error paths: invalid seat, focal mandate not in deck, insufficient undealt pool.
    assert_eq!(
        draw_scenarios(&obs.spec, focal_mandate, 4, 1, &pair_sha, 1, 2)
            .err()
            .map(|e| e.code()),
        Some("INVALID_FOCAL_SEAT")
    );
    let mut small_spec: PublicTableSpec = obs.spec.clone();
    small_spec.mandate_deck = vec![GradedMandateCard {
        card_id: 99,
        kind: MandateKind::MinProjectEquity { target: 999_999 },
        difficulty_completion_bps: 5_000,
        bonus_equity: 9_999,
    }];
    assert_eq!(
        draw_scenarios(&small_spec, focal_mandate, 0, 1, &pair_sha, 1, 2)
            .err()
            .map(|e| e.code()),
        Some("FOCAL_MANDATE_NOT_IN_DECK")
    );
    let matching_card = GradedMandateCard {
        card_id: 100,
        kind: focal_mandate.kind,
        difficulty_completion_bps: 5_000,
        bonus_equity: focal_mandate.bonus_equity,
    };
    small_spec.mandate_deck.push(matching_card);
    assert_eq!(
        draw_scenarios(&small_spec, focal_mandate, 0, 1, &pair_sha, 1, 2)
            .err()
            .map(|e| e.code()),
        Some("INSUFFICIENT_MANDATE_POOL")
    );
    Ok(())
}

#[test]
fn scoring_math_spans_flat_sets_drafts_ranks_split_half_and_prescreen() -> TestResult {
    let mut cfg = ScorerConfig::s1_standard(4);
    cfg.prescreen_min_arm_indep_candidates = 2;
    cfg.prescreen_min_arm_indep_span = 50;
    let cfg_sha = cfg.sha256_hex()?;
    let pair_sha = sha256_hex(b"math-pair");

    // Four candidates: c_passive (indep, in_span), c_synth (indep, in_span),
    // c_best_arm (committed, in_span), c_draft (first draft: !in_span, !is_arm_independent).
    let sub_passive = EvalActSubmission::default();
    let sub_synth = EvalActSubmission {
        market_actions: vec![MarketAction::TownImport {
            good: AssetId::Raw(0),
            seat: 0,
            amount: 10,
        }],
        pass_b_actions: Vec::new(),
        bundle: SealedOrderBundle::default(),
    };
    let sub_committed = EvalActSubmission {
        market_actions: vec![MarketAction::TownImport {
            good: AssetId::Raw(0),
            seat: 0,
            amount: 20,
        }],
        pass_b_actions: Vec::new(),
        bundle: SealedOrderBundle::default(),
    };
    let sub_draft = EvalActSubmission {
        market_actions: vec![MarketAction::TownImport {
            good: AssetId::Raw(0),
            seat: 0,
            amount: 30,
        }],
        pass_b_actions: Vec::new(),
        bundle: SealedOrderBundle::default(),
    };

    let c_passive = build_candidate(sub_passive, true, true)?;
    let c_synth = build_candidate(sub_synth, true, true)?;
    let c_committed = build_candidate(sub_committed, true, false)?;
    let c_draft = build_candidate(sub_draft, false, false)?;

    let mut tagged = vec![
        (
            c_passive.clone(),
            vec![
                ScenarioRolloutOutcome {
                    mandate_bonus: 0,
                    net_asset_equity: 100,
                    project_equity: 0,
                    total_equity: 100,
                };
                4
            ],
        ),
        (
            c_synth.clone(),
            [200_i64, 300, 100, 200]
                .into_iter()
                .map(|t| ScenarioRolloutOutcome {
                    mandate_bonus: 50,
                    net_asset_equity: t - 50,
                    project_equity: 0,
                    total_equity: t,
                })
                .collect(),
        ),
        (
            c_committed.clone(),
            [500_i64, 500, 100, 100]
                .into_iter()
                .map(|t| ScenarioRolloutOutcome {
                    mandate_bonus: 0,
                    net_asset_equity: 0,
                    project_equity: t,
                    total_equity: t,
                })
                .collect(),
        ),
        (
            c_draft.clone(),
            vec![
                ScenarioRolloutOutcome {
                    mandate_bonus: 0,
                    net_asset_equity: 900,
                    project_equity: 0,
                    total_equity: 900,
                };
                4
            ],
        ),
    ];
    tagged.sort_by(|(a, _), (b, _)| a.candidate_id.cmp(&b.candidate_id));
    let candidates: Vec<BlindCandidate> = tagged.iter().map(|(c, _)| c.clone()).collect();
    let outcomes: Vec<Vec<ScenarioRolloutOutcome>> = tagged.into_iter().map(|(_, o)| o).collect();

    let req = BlindScoringRequest {
        candidates,
        focal_seat: 0,
        pre_act_pair_sha256: pair_sha.clone(),
        pre_act_save_sha256: sha256_hex(b"save"),
        pre_act_seal_sha256: sha256_hex(b"seal"),
        schema_version: 1,
        scorer_config_sha256: cfg_sha.clone(),
        scorer_seed: 1,
        state_id: "hand_math_1".to_owned(),
        talk_commitments: Vec::new(),
        talk_extraction_counts: TalkExtractionCounts::default(),
    };

    let out = score_outcomes(&req, &cfg, 5, &outcomes)?;
    assert!(!out.degenerate_flat_set);
    assert_eq!(out.v_min, 100.0);
    assert_eq!(out.v_max, 300.0);
    assert_eq!(out.c_best, c_committed.candidate_id);

    let by_id: BTreeMap<String, _> = out
        .candidates
        .iter()
        .map(|c| (c.candidate_id.clone(), c))
        .collect();
    assert_eq!(by_id[&c_passive.candidate_id].ev_total_equity, 100.0);
    assert_eq!(by_id[&c_passive.candidate_id].score_norm, Some(0.0));
    assert_eq!(by_id[&c_passive.candidate_id].regret_equity, 200.0);
    assert_eq!(
        by_id[&c_passive.candidate_id].sensitivity_indep_rank,
        Some(2)
    );
    assert_eq!(
        by_id[&c_passive.candidate_id].sensitivity_indep_percentile,
        Some(0.25)
    );

    assert_eq!(by_id[&c_synth.candidate_id].ev_total_equity, 200.0);
    assert_eq!(by_id[&c_synth.candidate_id].score_norm, Some(0.5));
    assert_eq!(by_id[&c_synth.candidate_id].regret_equity, 100.0);
    assert_eq!(by_id[&c_synth.candidate_id].sensitivity_indep_rank, Some(1));
    assert_eq!(
        by_id[&c_synth.candidate_id].sensitivity_indep_percentile,
        Some(0.75)
    );

    assert_eq!(by_id[&c_committed.candidate_id].ev_total_equity, 300.0);
    assert_eq!(by_id[&c_committed.candidate_id].score_norm, Some(1.0));
    assert_eq!(by_id[&c_committed.candidate_id].regret_equity, 0.0);
    assert_eq!(by_id[&c_committed.candidate_id].paired_se_equity, 0.0);
    assert_eq!(
        by_id[&c_committed.candidate_id].sensitivity_indep_rank,
        Some(1)
    );
    assert_eq!(
        by_id[&c_committed.candidate_id].sensitivity_indep_percentile,
        Some(1.0)
    );

    // Draft (mean 900.0, !in_span) stays outside [v_min, v_max] and still gets scored.
    assert!(!by_id[&c_draft.candidate_id].in_span);
    assert_eq!(by_id[&c_draft.candidate_id].ev_total_equity, 900.0);
    assert_eq!(by_id[&c_draft.candidate_id].score_norm, Some(4.0));

    // Check paired_se_equity for c_passive vs c_best (diffs: [400, 400, 0, 0], mean 200, var = 160000/3, se = sqrt(160000/3)/2 = 200/sqrt(3)).
    let expected_se = 200.0 / 3.0_f64.sqrt();
    assert!((by_id[&c_passive.candidate_id].paired_se_equity - expected_se).abs() < 1e-9);

    // Regret >= 0 for every in_span candidate.
    for cand in &out.candidates {
        if cand.in_span {
            assert!(cand.regret_equity >= 0.0);
        }
    }

    // Split-half Winner's curse: H_A selects c_committed (mean 500 vs H_B 100 -> diff 400);
    // H_B selects c_synth (mean 150 vs H_A 250 -> diff -100); mean bias = 150.0.
    assert_eq!(out.winners_curse.half_scenarios, 2);
    assert_eq!(out.winners_curse.c_star_half_a, c_committed.candidate_id);
    assert_eq!(out.winners_curse.c_star_half_b, c_synth.candidate_id);
    assert_eq!(out.winners_curse.diff_a_to_b_equity, 400.0);
    assert_eq!(out.winners_curse.diff_b_to_a_equity, -100.0);
    assert_eq!(out.winners_curse.bias_estimate_equity, 150.0);
    assert!(out.winners_curse.is_half_m_upper_estimate);

    assert!(!out.prescreen.excluded_by_prescreen);
    assert!(!out.prescreen.is_flat_indep);
    assert_eq!(out.prescreen.n_feasible_indep, 2);
    assert_eq!(out.prescreen.span_indep, 100.0);

    // Pre-screen excludes when threshold is raised above span_indep = 100.0.
    let mut strict_cfg = cfg.clone();
    strict_cfg.prescreen_min_arm_indep_span = 101;
    let mut req_strict = req.clone();
    req_strict.scorer_config_sha256 = strict_cfg.sha256_hex()?;
    let out_strict = score_outcomes(&req_strict, &strict_cfg, 5, &outcomes)?;
    assert!(out_strict.prescreen.excluded_by_prescreen);

    // Flat span case -> score_norm is None (`null` in JSON, never 1.0) & degenerate_flat_set: true.
    let flat_outcomes = vec![
        vec![
            ScenarioRolloutOutcome {
                mandate_bonus: 0,
                net_asset_equity: 250,
                project_equity: 0,
                total_equity: 250,
            };
            4
        ];
        4
    ];
    let flat_out = score_outcomes(&req, &cfg, 5, &flat_outcomes)?;
    assert!(flat_out.degenerate_flat_set);
    assert_eq!(flat_out.v_min, 250.0);
    assert_eq!(flat_out.v_max, 250.0);
    assert!(flat_out.prescreen.is_flat_indep);
    assert!(flat_out.prescreen.excluded_by_prescreen);
    for c in &flat_out.candidates {
        assert_eq!(c.score_norm, None);
    }
    let flat_json = to_sorted_pretty_json(&flat_out)?;
    assert!(flat_json.contains("\"score_norm\": null"));
    Ok(())
}

#[test]
fn real_checkpoint_rollout_cli_determinism_and_engine_parity() -> TestResult {
    let tmp = UniqueTmpDir::new("rollout_cli")?;
    let TestCheckpointFixture {
        cas_save,
        cas_seal,
        save_sha,
        seal_sha,
        pair_sha,
        save_bytes: _,
    } = make_real_checkpoint_pair(tmp.path(), 8102, 0)?;
    assert!(probe_checkpoint_sanitized(&cas_save, &cas_seal).is_ok());

    let cfg = ScorerConfig::s1_standard(2);
    let cfg_sha = cfg.sha256_hex()?;
    let cfg_path = tmp.path().join("scorer_config.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;

    let c_passive = build_candidate(EvalActSubmission::default(), true, true)?;
    let c_import = build_candidate(
        EvalActSubmission {
            market_actions: vec![MarketAction::TownImport {
                good: AssetId::Raw(0),
                seat: 0,
                amount: 400,
            }],
            pass_b_actions: Vec::new(),
            bundle: SealedOrderBundle::default(),
        },
        true,
        false,
    )?;
    let candidates = build_sorted_candidates(vec![c_passive.clone(), c_import.clone()]);
    let req = BlindScoringRequest {
        candidates,
        focal_seat: 0,
        pre_act_pair_sha256: pair_sha.clone(),
        pre_act_save_sha256: save_sha.clone(),
        pre_act_seal_sha256: seal_sha.clone(),
        schema_version: 1,
        scorer_config_sha256: cfg_sha.clone(),
        scorer_seed: 12345,
        state_id: "season8102_day-01_seat-0".to_owned(),
        talk_commitments: Vec::new(),
        talk_extraction_counts: TalkExtractionCounts::default(),
    };
    verify_checkpoint_pair(&cas_save, &cas_seal, &req)?;

    // Check direct rollout vs manual EvalHost replay parity for both candidates.
    let probe = EvalHost::fork_from_checkpoint(&cas_save, &cas_seal)?;
    let focal_obs = probe.observe_seat(Seat(0))?;
    let focal_mandate = *focal_obs.own_mandate().ok_or("missing focal own_mandate")?;
    let scenarios = draw_scenarios(&focal_obs.spec, focal_mandate, 0, 12345, &pair_sha, 1, 2)?;
    let mut peer_subs_0 = BTreeMap::new();
    for s in 1..4_u8 {
        let p_obs = probe.observe_seat(Seat(s))?;
        let p_mandate = *scenarios[0]
            .peer_mandates
            .get(&s)
            .ok_or("missing peer mandate")?;
        let dec = ClubBot::new(scenarios[0].bot_base_seed.wrapping_add(u64::from(s)))
            .decide_full_turn(&p_obs, &p_mandate);
        peer_subs_0.insert(
            s,
            EvalActSubmission {
                market_actions: dec.pass_a_actions,
                pass_b_actions: dec.pass_b_actions,
                bundle: dec.bundle,
            },
        );
    }

    let mut rollout_results = Vec::new();
    for sub in [&c_passive.submission, &c_import.submission] {
        let rollout_res = run_single_rollout(
            &cas_save,
            &cas_seal,
            (&save_sha, &seal_sha, &pair_sha),
            0,
            sub,
            &peer_subs_0,
            focal_mandate,
            &scenarios[0],
            PostureKind::Standard,
            &cfg,
        )?;
        rollout_results.push(rollout_res);

        let mut manual = EvalHost::fork_from_checkpoint(&cas_save, &cas_seal)?;
        let mut day1 = peer_subs_0.clone();
        day1.insert(0, sub.clone());
        manual.apply_act_submissions(&day1)?;
        manual.sweep_turn()?;
        while manual.phase() == TurnPhase::TalkAndMarket {
            let mut subs = BTreeMap::new();
            for s in 0..4_u8 {
                let obs_s = manual.observe_seat(Seat(s))?;
                let m_s = if s == 0 {
                    focal_mandate
                } else {
                    *scenarios[0]
                        .peer_mandates
                        .get(&s)
                        .ok_or("missing peer mandate")?
                };
                let dec = ClubBot::new(scenarios[0].bot_base_seed.wrapping_add(u64::from(s)))
                    .decide_full_turn(&obs_s, &m_s);
                subs.insert(
                    s,
                    EvalActSubmission {
                        market_actions: dec.pass_a_actions,
                        pass_b_actions: dec.pass_b_actions,
                        bundle: dec.bundle,
                    },
                );
            }
            manual.apply_act_submissions(&subs)?;
            manual.sweep_turn()?;
        }
        let (sb, log, _, _) = manual.finalize_eval_season()?;
        let replayed = replay_eval_season(&log)?;
        let row0 = sb
            .standings
            .iter()
            .find(|e| e.seat == 0)
            .ok_or("missing seat 0")?;
        let replay_row0 = replayed
            .scoreboard
            .standings
            .iter()
            .find(|e| e.seat == 0)
            .ok_or("missing replay seat 0")?;
        assert_eq!(rollout_res.total_equity, row0.total_equity);
        assert_eq!(rollout_res.project_equity, row0.project_equity);
        assert_eq!(rollout_res.net_asset_equity, row0.net_asset_equity);
        assert_eq!(rollout_res.mandate_bonus, row0.mandate_bonus);
        assert_eq!(rollout_res.total_equity, replay_row0.total_equity);
        assert_eq!(rollout_res.project_equity, replay_row0.project_equity);
        assert_eq!(rollout_res.net_asset_equity, replay_row0.net_asset_equity);
        assert_eq!(rollout_res.mandate_bonus, replay_row0.mandate_bonus);
    }
    assert_ne!(
        rollout_results[0], rollout_results[1],
        "distinct focal submissions should produce distinct rollout outcomes"
    );

    // Also run `execute_blind_request` and verify CLI two-pass byte-identity.
    let direct_out = execute_blind_request(&req, &cfg, &cas_save, &cas_seal)?;
    let blind_dir = tmp.path().join("blind_inputs");
    fs::create_dir_all(&blind_dir)?;
    let req_compact_bytes = to_sorted_compact_json(&req)?;
    let expected_req_sha = sha256_hex(&req_compact_bytes);
    fs::write(
        blind_dir.join(format!("{}.json", req.state_id)),
        req_compact_bytes,
    )?;

    let out_a = tmp.path().join("out_a");
    let out_b = tmp.path().join("out_b");
    let ckpts_dir = tmp.path().join("checkpoints");

    let mut stdout_a = Vec::new();
    let mut stderr_a = Vec::new();
    let code_a = run_cli(
        [
            "score",
            "--blind-inputs",
            blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--out",
            out_a.to_str().ok_or("utf8")?,
        ],
        &mut stdout_a,
        &mut stderr_a,
    );
    assert_eq!(
        code_a,
        EXIT_OK,
        "stderr: {}",
        String::from_utf8_lossy(&stderr_a)
    );

    let mut stdout_b = Vec::new();
    let mut stderr_b = Vec::new();
    let code_b = run_cli(
        [
            "score",
            "--blind-inputs",
            blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--out",
            out_b.to_str().ok_or("utf8")?,
        ],
        &mut stdout_b,
        &mut stderr_b,
    );
    assert_eq!(code_b, EXIT_OK);
    assert_eq!(stdout_a, stdout_b);
    assert_eq!(String::from_utf8_lossy(&stdout_a).trim(), "scored_states=1");

    let bytes_a = fs::read(out_a.join("scores/season8102_day-01_seat-0.json"))?;
    let bytes_b = fs::read(out_b.join("scores/season8102_day-01_seat-0.json"))?;
    assert_eq!(
        bytes_a, bytes_b,
        "CLI score output must be byte-identical across runs"
    );
    let text_a = String::from_utf8(bytes_a)?;
    eprintln!("EXAMPLE_SCORE_FILE_JSON:\n{text_a}");
    let parsed: StateScoreOutput = serde_json::from_str(&text_a)?;
    assert_eq!(parsed.blind_request_sha256, expected_req_sha);
    let mut direct_aligned = direct_out;
    direct_aligned.blind_request_sha256 = expected_req_sha;
    assert_eq!(parsed, direct_aligned);

    // Confirm sorted object keys at root and nested levels, and zero disallowed fields.
    let val_a: serde_json::Value = serde_json::from_str(&text_a)?;
    let root_keys: Vec<&String> = val_a.as_object().ok_or("root object")?.keys().collect();
    let mut sorted_keys = root_keys.clone();
    sorted_keys.sort();
    assert_eq!(root_keys, sorted_keys);
    for forbidden in [
        "timestamp",
        "duration",
        "hostname",
        "checkpoints",
        "setup.json",
        "arm_map",
    ] {
        assert!(
            !text_a.contains(forbidden),
            "unexpected `{forbidden}` in score output"
        );
    }

    // Verify compiled binary runs `--help`, `--version`, and `score`.
    let bin = env!("CARGO_BIN_EXE_league-scorer");
    let out_c = tmp.path().join("out_c");
    let proc_out = Command::new(bin)
        .args([
            "score",
            "--blind-inputs",
            blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--out",
            out_c.to_str().ok_or("utf8")?,
        ])
        .output()?;
    assert!(proc_out.status.success());
    let bytes_c = fs::read(out_c.join("scores/season8102_day-01_seat-0.json"))?;
    assert_eq!(bytes_c, text_a.as_bytes());
    Ok(())
}

#[test]
fn cli_refuses_bad_layouts_tampered_checkpoints_and_usage_errors() -> TestResult {
    let tmp = UniqueTmpDir::new("cli_errors")?;
    let TestCheckpointFixture {
        cas_save: _,
        cas_seal: _,
        save_sha,
        seal_sha,
        pair_sha,
        save_bytes,
    } = make_real_checkpoint_pair(tmp.path(), 8103, 0)?;

    let cfg = ScorerConfig::s1_standard(2);
    let cfg_sha = cfg.sha256_hex()?;
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;

    let c_passive = build_candidate(EvalActSubmission::default(), true, true)?;
    let req = BlindScoringRequest {
        candidates: vec![c_passive],
        focal_seat: 0,
        pre_act_pair_sha256: pair_sha,
        pre_act_save_sha256: save_sha.clone(),
        pre_act_seal_sha256: seal_sha.clone(),
        schema_version: 1,
        scorer_config_sha256: cfg_sha.clone(),
        scorer_seed: 7,
        state_id: "state_8103".to_owned(),
        talk_commitments: Vec::new(),
        talk_extraction_counts: TalkExtractionCounts::default(),
    };
    let blind_dir = tmp.path().join("blind");
    fs::create_dir_all(&blind_dir)?;
    let empty_blind_dir = tmp.path().join("blind_empty");
    fs::create_dir_all(&empty_blind_dir)?;
    let ckpts_dir = tmp.path().join("checkpoints");
    let out_dir = tmp.path().join("out");

    // Help / version / usage errors.
    for args in [
        vec!["--help"],
        vec!["--version"],
        vec![],
        vec!["unknown"],
        vec!["--error-format", "bad"],
        vec!["--error-format"],
        vec!["score"],
        vec!["score", "--blind-inputs", "a", "--blind-inputs", "b"],
        vec!["score", "--unknown"],
        vec![
            "score",
            "--blind-inputs",
            "a",
            "--checkpoints",
            "b",
            "--config",
            "c",
            "--out",
            "d",
            "--error-format",
            "json",
        ],
    ] {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let rc = run_cli(args.clone(), &mut out, &mut err);
        if args == vec!["--help"] || args == vec!["--version"] {
            assert_eq!(rc, EXIT_OK);
            assert!(!out.is_empty());
        } else {
            assert_eq!(rc, EXIT_USER_ERROR, "args={args:?}");
            assert!(!err.is_empty());
        }
    }

    // Empty blind inputs refused.
    let mut out = Vec::new();
    let mut err = Vec::new();
    let rc_empty = run_cli(
        [
            "--error-format",
            "json",
            "score",
            "--blind-inputs",
            empty_blind_dir.to_str().ok_or("utf8")?,
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--out",
            out_dir.to_str().ok_or("utf8")?,
        ],
        &mut out,
        &mut err,
    );
    assert_eq!(rc_empty, EXIT_USER_ERROR);
    let err_s = String::from_utf8_lossy(&err);
    assert!(err_s.contains("NO_BLIND_INPUTS"), "err={err_s}");

    // Stem mismatch refused with STATE_ID_FILENAME_MISMATCH, then duplicate / case-colliding state_ids refused.
    fs::write(blind_dir.join("01.json"), to_sorted_compact_json(&req)?)?;
    let mut err_stem = Vec::new();
    assert_eq!(
        run_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_stem,
        ),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_stem).contains("STATE_ID_FILENAME_MISMATCH"));
    fs::remove_file(blind_dir.join("01.json"))?;

    fs::write(
        blind_dir.join(format!("{}.json", req.state_id)),
        to_sorted_compact_json(&req)?,
    )?;
    #[cfg(unix)]
    {
        let mut req_upper = req.clone();
        req_upper.state_id = "State_8103".to_owned();
        let upper_path = blind_dir.join("State_8103.json");
        fs::write(&upper_path, to_sorted_compact_json(&req_upper)?)?;
        let mut err_dup = Vec::new();
        assert_eq!(
            run_cli(
                [
                    "score",
                    "--blind-inputs",
                    blind_dir.to_str().ok_or("utf8")?,
                    "--checkpoints",
                    ckpts_dir.to_str().ok_or("utf8")?,
                    "--config",
                    cfg_path.to_str().ok_or("utf8")?,
                    "--out",
                    out_dir.to_str().ok_or("utf8")?,
                ],
                &mut Vec::new(),
                &mut err_dup,
            ),
            EXIT_USER_ERROR
        );
        assert!(String::from_utf8_lossy(&err_dup).contains("INVALID_STATE_ID"));
        fs::remove_file(upper_path)?;
    }

    // Tampered save bytes refused with CHECKPOINT_HASH_MISMATCH before fork.
    let cas_save = ckpts_dir.join(format!("{save_sha}.save.json"));
    let mut tampered = save_bytes.clone();
    tampered.push(b' ');
    fs::write(&cas_save, &tampered)?;
    let mut err_tamper = Vec::new();
    assert_eq!(
        run_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_tamper,
        ),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_tamper).contains("CHECKPOINT_HASH_MISMATCH"));
    fs::write(&cas_save, &save_bytes)?;

    // Wrong pair hash refused.
    let mut bad_pair_req = req.clone();
    bad_pair_req.pre_act_pair_sha256 = sha256_hex(b"wrong-pair");
    assert_eq!(
        verify_checkpoint_pair(
            &cas_save,
            &ckpts_dir.join(format!("{seal_sha}.seal")),
            &bad_pair_req
        )
        .err()
        .map(|e| e.code()),
        Some("CHECKPOINT_HASH_MISMATCH")
    );

    // Missing seal refused with CHECKPOINT_NOT_FOUND.
    fs::remove_file(ckpts_dir.join(format!("{seal_sha}.seal")))?;
    let mut err_missing = Vec::new();
    assert_eq!(
        run_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_missing,
        ),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_missing).contains("CHECKPOINT_NOT_FOUND"));

    // Disallowed non-hash filename in --checkpoints refused with INVALID_CHECKPOINTS_LAYOUT.
    fs::write(ckpts_dir.join("setup.json"), b"{}")?;
    let mut err_layout = Vec::new();
    assert_eq!(
        run_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_layout,
        ),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_layout).contains("INVALID_CHECKPOINTS_LAYOUT"));
    must_ok(
        fs::remove_file(ckpts_dir.join("setup.json")),
        "rm setup.json",
    );

    // Subdirectory inside --checkpoints refused.
    fs::create_dir_all(ckpts_dir.join("nested"))?;
    let mut err_sub = Vec::new();
    assert_eq!(
        run_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_sub,
        ),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_sub).contains("INVALID_CHECKPOINTS_LAYOUT"));
    Ok(())
}

struct FailingWriter;

impl std::io::Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("simulated write error"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn all_scorer_error_variants_report_codes_hints_exit_codes_and_display() -> TestResult {
    let variants = vec![
        (ScorerError::Usage("bad usage".into()), EXIT_USER_ERROR),
        (ScorerError::UnsupportedSchemaVersion(99), EXIT_USER_ERROR),
        (
            ScorerError::InvalidStateId("bad id".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::InvalidHexDigest {
                field: "candidate_id",
                value: "zz".into(),
            },
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::InvalidFocalSeat {
                focal_seat: 4,
                seat_count: 4,
            },
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::EmptyCandidates("empty".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::NoInSpanCandidates("none".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::UnsortedCandidates {
                prev: "b".into(),
                current: "a".into(),
            },
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::DuplicateCandidateId("a".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::CandidateIdMismatch {
                index: 0,
                declared: "a".into(),
                actual: "b".into(),
            },
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::ConfigHashMismatch {
                expected: "a".into(),
                actual: "b".into(),
            },
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::StateIdFilenameMismatch("state_1 != wrong".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::UnknownSubmissionField("extra field".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::InvalidTalkCommitment("bad actor".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::InvalidScorerConfig("odd M".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::FreezeConfigMismatch("cfg mismatch".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::FreezeTreeMismatch("tree mismatch".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::FreezeManifestMismatch("manifest mismatch".into()),
            EXIT_USER_ERROR,
        ),
        (ScorerError::Json("parse".into()), EXIT_USER_ERROR),
        (ScorerError::NoBlindInputs("none".into()), EXIT_USER_ERROR),
        (
            ScorerError::InvalidCheckpointsLayout("bad".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::CheckpointNotFound("missing".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::CheckpointHashMismatch("mismatch".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::InvalidCheckpointState("5 seats".into()),
            EXIT_USER_ERROR,
        ),
        (ScorerError::FocalMandateNotInDeck, EXIT_RUNTIME_ERROR),
        (
            ScorerError::InsufficientMandatePool {
                pool_len: 1,
                needed: 3,
            },
            EXIT_RUNTIME_ERROR,
        ),
        (ScorerError::Engine("engine".into()), EXIT_RUNTIME_ERROR),
        (ScorerError::Io("disk".into()), EXIT_RUNTIME_ERROR),
    ];

    for (err, expected_exit) in variants {
        let code = err.code();
        assert!(!code.is_empty());
        assert_eq!(err.exit_code(), expected_exit, "code={code}");
        assert!(!err.hint().is_empty(), "code={code}");
        let msg = err.to_string();
        assert!(msg.starts_with(code), "msg={msg}");
        let structured = err.to_structured_error();
        assert_eq!(structured.code, code);
        assert_eq!(structured.hint, err.hint());
    }

    assert_eq!(
        league_scorer::validate_state_id(".")
            .err()
            .map(|e| e.code()),
        Some("INVALID_STATE_ID")
    );
    let too_long = "a".repeat(129);
    assert_eq!(
        league_scorer::validate_state_id(&too_long)
            .err()
            .map(|e| e.code()),
        Some("INVALID_STATE_ID")
    );

    let mut cfg_nan = ScorerConfig::s1_standard(4);
    cfg_nan.p_honour = f64::NAN;
    assert_eq!(
        cfg_nan.validate().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );
    Ok(())
}

#[test]
fn rollout_and_scoring_edge_cases_are_refused_cleanly() -> TestResult {
    let tmp = UniqueTmpDir::new("rollout_edges")?;
    let TestCheckpointFixture {
        cas_save,
        cas_seal,
        save_sha,
        seal_sha,
        pair_sha,
        save_bytes: _,
    } = make_real_checkpoint_pair(tmp.path(), 8108, 0)?;

    let cfg = ScorerConfig::s1_standard(2);
    let cfg_sha = cfg.sha256_hex()?;
    let c_passive = build_candidate(EvalActSubmission::default(), true, false)?;
    let req = BlindScoringRequest {
        candidates: vec![c_passive.clone()],
        focal_seat: 0,
        pre_act_pair_sha256: pair_sha.clone(),
        pre_act_save_sha256: save_sha,
        pre_act_seal_sha256: seal_sha,
        schema_version: 1,
        scorer_config_sha256: cfg_sha.clone(),
        scorer_seed: 12,
        state_id: "edge_8108".to_owned(),
        talk_commitments: Vec::new(),
        talk_extraction_counts: TalkExtractionCounts::default(),
    };

    // Missing save file in verify_checkpoint_pair.
    assert_eq!(
        verify_checkpoint_pair(&tmp.path().join("missing.save.json"), &cas_seal, &req)
            .err()
            .map(|e| e.code()),
        Some("CHECKPOINT_NOT_FOUND")
    );

    // Seal hash mismatch in verify_checkpoint_pair.
    let bad_seal_path = tmp.path().join("tampered.seal");
    fs::write(&bad_seal_path, b"corrupt seal bytes")?;
    assert_eq!(
        verify_checkpoint_pair(&cas_save, &bad_seal_path, &req)
            .err()
            .map(|e| e.code()),
        Some("CHECKPOINT_HASH_MISMATCH")
    );

    // run_single_rollout errors: missing file, focal_seat out of range, illegal submission.
    let probe = EvalHost::fork_from_checkpoint(&cas_save, &cas_seal)?;
    let focal_obs = probe.observe_seat(Seat(0))?;
    let focal_mandate = *focal_obs.own_mandate().ok_or("missing focal mandate")?;
    let scenarios = draw_scenarios(&focal_obs.spec, focal_mandate, 0, 12, &pair_sha, 1, 2)?;
    let mut peer_subs = BTreeMap::new();
    for s in 1..4_u8 {
        let p_obs = probe.observe_seat(Seat(s))?;
        let p_mandate = *scenarios[0]
            .peer_mandates
            .get(&s)
            .ok_or("missing peer mandate")?;
        let dec = ClubBot::new(scenarios[0].bot_base_seed.wrapping_add(u64::from(s)))
            .decide_full_turn(&p_obs, &p_mandate);
        peer_subs.insert(
            s,
            EvalActSubmission {
                market_actions: dec.pass_a_actions,
                pass_b_actions: dec.pass_b_actions,
                bundle: dec.bundle,
            },
        );
    }

    let digests = (
        req.pre_act_save_sha256.as_str(),
        req.pre_act_seal_sha256.as_str(),
        req.pre_act_pair_sha256.as_str(),
    );

    assert_eq!(
        run_single_rollout(
            &tmp.path().join("missing.save.json"),
            &cas_seal,
            digests,
            0,
            &c_passive.submission,
            &peer_subs,
            focal_mandate,
            &scenarios[0],
            PostureKind::Standard,
            &cfg,
        )
        .err()
        .map(|e| e.code()),
        Some("CHECKPOINT_NOT_FOUND")
    );

    assert_eq!(
        run_single_rollout(
            &cas_save,
            &cas_seal,
            digests,
            4,
            &c_passive.submission,
            &peer_subs,
            focal_mandate,
            &scenarios[0],
            PostureKind::Standard,
            &cfg,
        )
        .err()
        .map(|e| e.code()),
        Some("INVALID_FOCAL_SEAT")
    );

    let mut bad_peer_subs = peer_subs.clone();
    bad_peer_subs.insert(99, EvalActSubmission::default());
    assert_eq!(
        run_single_rollout(
            &cas_save,
            &cas_seal,
            digests,
            0,
            &c_passive.submission,
            &bad_peer_subs,
            focal_mandate,
            &scenarios[0],
            PostureKind::Standard,
            &cfg,
        )
        .err()
        .map(|e| e.code()),
        Some("ENGINE_ERROR")
    );

    // 5-seat checkpoint refused by both run_single_rollout and execute_blind_request.
    let mut host5 = EvalHost::from_standard_setup(TableId(8109), 5, 1, 0)?
        .with_engine_commit(TEST_ENGINE_COMMIT)?;
    let raw5_dir = tmp.path().join("raw5");
    fs::create_dir_all(&raw5_dir)?;
    let (save5, seal5) = host5.write_pre_act_checkpoint(&raw5_dir)?;
    let save5_bytes = fs::read(&save5)?;
    let seal5_bytes = fs::read(&seal5)?;
    let req5 = BlindScoringRequest {
        candidates: vec![build_candidate(EvalActSubmission::default(), true, true)?],
        focal_seat: 0,
        pre_act_pair_sha256: compute_pre_act_pair_sha256_hex(&save5_bytes, &seal5_bytes),
        pre_act_save_sha256: sha256_hex(&save5_bytes),
        pre_act_seal_sha256: sha256_hex(&seal5_bytes),
        schema_version: 1,
        scorer_config_sha256: cfg_sha.clone(),
        scorer_seed: 1,
        state_id: "seat5".to_owned(),
        talk_commitments: Vec::new(),
        talk_extraction_counts: TalkExtractionCounts::default(),
    };
    assert_eq!(
        execute_blind_request(&req5, &cfg, &save5, &seal5)
            .err()
            .map(|e| e.code()),
        Some("INVALID_CHECKPOINT_STATE")
    );
    assert_eq!(
        run_single_rollout(
            &save5,
            &seal5,
            (
                &req5.pre_act_save_sha256,
                &req5.pre_act_seal_sha256,
                &req5.pre_act_pair_sha256,
            ),
            0,
            &c_passive.submission,
            &peer_subs,
            focal_mandate,
            &scenarios[0],
            PostureKind::Standard,
            &cfg,
        )
        .err()
        .map(|e| e.code()),
        Some("INVALID_CHECKPOINT_STATE")
    );

    // execute_blind_request refuses corrupt JSON with valid hashes as ENGINE_ERROR.
    let fake_save = tmp.path().join("fake.save.json");
    let fake_seal = tmp.path().join("fake.seal");
    fs::write(&fake_save, b"{not valid json")?;
    fs::write(&fake_seal, b"{not valid seal")?;
    let fake_req = BlindScoringRequest {
        candidates: vec![build_candidate(EvalActSubmission::default(), true, true)?],
        focal_seat: 0,
        pre_act_pair_sha256: compute_pre_act_pair_sha256_hex(
            b"{not valid json",
            b"{not valid seal",
        ),
        pre_act_save_sha256: sha256_hex(b"{not valid json"),
        pre_act_seal_sha256: sha256_hex(b"{not valid seal"),
        schema_version: 1,
        scorer_config_sha256: cfg_sha.clone(),
        scorer_seed: 1,
        state_id: "fake_ckpt".to_owned(),
        talk_commitments: Vec::new(),
        talk_extraction_counts: TalkExtractionCounts::default(),
    };
    assert_eq!(
        execute_blind_request(&fake_req, &cfg, &fake_save, &fake_seal)
            .err()
            .map(|e| e.code()),
        Some("ENGINE_ERROR")
    );

    // score_outcomes shape mismatches & zero arm-independent candidates branch.
    assert_eq!(
        score_outcomes(&req, &cfg, 1, &[]).err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );
    assert_eq!(
        score_outcomes(
            &req,
            &cfg,
            1,
            &[vec![ScenarioRolloutOutcome {
                mandate_bonus: 0,
                net_asset_equity: 0,
                project_equity: 0,
                total_equity: 0,
            }]],
        )
        .err()
        .map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );
    let no_indep_out = score_outcomes(
        &req,
        &cfg,
        1,
        &[vec![
            ScenarioRolloutOutcome {
                mandate_bonus: 0,
                net_asset_equity: 100,
                project_equity: 0,
                total_equity: 100,
            },
            ScenarioRolloutOutcome {
                mandate_bonus: 0,
                net_asset_equity: 200,
                project_equity: 0,
                total_equity: 200,
            },
        ]],
    )?;
    assert_eq!(no_indep_out.prescreen.n_feasible_indep, 0);
    assert_eq!(no_indep_out.prescreen.v_min_indep, None);
    assert_eq!(no_indep_out.prescreen.v_max_indep, None);
    assert!(no_indep_out.prescreen.excluded_by_prescreen);
    assert_eq!(no_indep_out.candidates[0].sensitivity_indep_rank, None);
    assert_eq!(
        no_indep_out.candidates[0].sensitivity_indep_percentile,
        None
    );
    Ok(())
}

#[test]
fn cli_covers_all_flag_errors_io_failures_and_formats() -> TestResult {
    let tmp = UniqueTmpDir::new("cli_more_edges")?;
    let TestCheckpointFixture {
        cas_save: _,
        cas_seal: _,
        save_sha,
        seal_sha,
        pair_sha,
        save_bytes: _,
    } = make_real_checkpoint_pair(tmp.path(), 8110, 0)?;

    let cfg = ScorerConfig::s1_standard(2);
    let cfg_sha = cfg.sha256_hex()?;
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;

    let c_passive = build_candidate(EvalActSubmission::default(), true, true)?;
    let req = BlindScoringRequest {
        candidates: vec![c_passive],
        focal_seat: 0,
        pre_act_pair_sha256: pair_sha,
        pre_act_save_sha256: save_sha,
        pre_act_seal_sha256: seal_sha,
        schema_version: 1,
        scorer_config_sha256: cfg_sha.clone(),
        scorer_seed: 9,
        state_id: "state_8110".to_owned(),
        talk_commitments: Vec::new(),
        talk_extraction_counts: TalkExtractionCounts::default(),
    };

    let blind_dir = tmp.path().join("blind");
    fs::create_dir_all(&blind_dir)?;
    fs::write(blind_dir.join("notes.txt"), b"ignored")?;
    fs::create_dir_all(blind_dir.join("nested_dir"))?;
    fs::write(
        blind_dir.join(format!("{}.json", req.state_id)),
        to_sorted_compact_json(&req)?,
    )?;
    let ckpts_dir = tmp.path().join("checkpoints");
    let out_dir = tmp.path().join("out");

    // Short flags `-h` and `-V` (plus `--help` source-tree hash disclosure), and stdout write failures.
    for flag in ["-h", "--help"] {
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(run_cli([flag], &mut out, &mut err), EXIT_OK);
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("--freeze <file>"),
            "help missing --freeze: {text}"
        );
        assert!(
            text.contains("--scorer-crate-dir <dir>"),
            "help missing --scorer-crate-dir: {text}"
        );
        assert!(
            text.contains("source tree") && text.contains("not the running binary"),
            "help missing source-tree vs running-binary disclosure: {text}"
        );
    }
    for flag in ["-h", "-V"] {
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(run_cli([flag], &mut out, &mut err), EXIT_OK);
        assert!(!out.is_empty());

        let mut fail_out = FailingWriter;
        let mut err_w = Vec::new();
        assert_eq!(
            run_cli([flag], &mut fail_out, &mut err_w),
            EXIT_RUNTIME_ERROR
        );
    }

    // Usage edge cases (duplicate flags, missing values, empty value, missing required flags, text error format).
    for args in [
        vec!["--error-format", "text"],
        vec!["score", "--blind-inputs"],
        vec!["score", "--blind-inputs", ""],
        vec!["score", "--checkpoints", "a", "--checkpoints", "b"],
        vec!["score", "--config", "a", "--config", "b"],
        vec!["score", "--out", "a", "--out", "b"],
        vec!["score", "--error-format"],
        vec!["score", "--error-format", "xml"],
        vec!["score", "--blind-inputs", "a"],
        vec!["score", "--blind-inputs", "a", "--checkpoints", "b"],
        vec![
            "score",
            "--blind-inputs",
            "a",
            "--checkpoints",
            "b",
            "--config",
            "c",
        ],
        vec![
            "score",
            "--blind-inputs",
            "a",
            "--checkpoints",
            "b",
            "--config",
            "c",
            "--out",
            "d",
            "--error-format",
            "text",
        ],
    ] {
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(
            run_cli(args.clone(), &mut out, &mut err),
            EXIT_USER_ERROR,
            "args={args:?}"
        );
        assert!(!err.is_empty());
    }

    // Missing config file & bad JSON config file.
    let bad_cfg_path = tmp.path().join("bad_cfg.json");
    fs::write(&bad_cfg_path, b"{bad")?;
    for (cfg_arg, expected_code) in [
        (tmp.path().join("no_cfg.json"), "INVALID_SCORER_CONFIG"),
        (bad_cfg_path, "INVALID_JSON"),
    ] {
        let mut err = Vec::new();
        assert_eq!(
            run_cli(
                [
                    "score",
                    "--blind-inputs",
                    blind_dir.to_str().ok_or("utf8")?,
                    "--checkpoints",
                    ckpts_dir.to_str().ok_or("utf8")?,
                    "--config",
                    cfg_arg.to_str().ok_or("utf8")?,
                    "--out",
                    out_dir.to_str().ok_or("utf8")?,
                ],
                &mut Vec::new(),
                &mut err,
            ),
            EXIT_USER_ERROR
        );
        assert!(String::from_utf8_lossy(&err).contains(expected_code));
    }

    // Missing checkpoints dir & Missing blind-inputs dir.
    for (bi, ck, expected_code) in [
        (
            &blind_dir,
            &tmp.path().join("no_ckpts"),
            "INVALID_CHECKPOINTS_LAYOUT",
        ),
        (&tmp.path().join("no_blind"), &ckpts_dir, "NO_BLIND_INPUTS"),
    ] {
        let mut err = Vec::new();
        assert_eq!(
            run_cli(
                [
                    "score",
                    "--blind-inputs",
                    bi.to_str().ok_or("utf8")?,
                    "--checkpoints",
                    ck.to_str().ok_or("utf8")?,
                    "--config",
                    cfg_path.to_str().ok_or("utf8")?,
                    "--out",
                    out_dir.to_str().ok_or("utf8")?,
                ],
                &mut Vec::new(),
                &mut err,
            ),
            EXIT_USER_ERROR
        );
        assert!(String::from_utf8_lossy(&err).contains(expected_code));
    }

    // Non-64-hex `.save.json` stem inside `--checkpoints`.
    let non_hex = ckpts_dir.join("short.save.json");
    fs::write(&non_hex, b"{}")?;
    let mut err_non_hex = Vec::new();
    assert_eq!(
        run_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_non_hex,
        ),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_non_hex).contains("INVALID_CHECKPOINTS_LAYOUT"));
    fs::remove_file(&non_hex)?;

    // Invalid JSON blind request.
    let bad_req_path = blind_dir.join("zzz_bad.json");
    fs::write(&bad_req_path, b"{not json")?;
    let mut err_bad_req = Vec::new();
    assert_eq!(
        run_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_bad_req,
        ),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_bad_req).contains("INVALID_JSON"));
    fs::remove_file(&bad_req_path)?;

    // `--out` pointing to a regular file fails `create_dir_all` with EXIT_RUNTIME_ERROR.
    let file_as_out = tmp.path().join("out_is_file");
    fs::write(&file_as_out, b"block")?;
    let mut err_out_io = Vec::new();
    assert_eq!(
        run_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                file_as_out.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_out_io,
        ),
        EXIT_RUNTIME_ERROR
    );
    assert!(String::from_utf8_lossy(&err_out_io).contains("IO_ERROR"));

    // Valid execution succeeds even with ignored notes.txt / nested_dir, and handles stdout write error.
    let mut fail_stdout = FailingWriter;
    let mut err_stdout = Vec::new();
    assert_eq!(
        run_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut fail_stdout,
            &mut err_stdout,
        ),
        EXIT_RUNTIME_ERROR
    );
    assert!(String::from_utf8_lossy(&err_stdout).contains("IO_ERROR"));

    // `--freeze <file>` succeeds when valid (ignoring extra unblinded fields) and refuses mismatches.
    let scorer_manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let real_tree_sha = compute_scorer_tree_sha256(scorer_manifest_dir)?;
    let real_manifest_sha = compute_blind_inputs_manifest_sha256(&blind_dir)?;
    let save_bytes_for_vc =
        fs::read(ckpts_dir.join(format!("{}.save.json", req.pre_act_save_sha256)))?;
    let seal_bytes_for_vc = fs::read(ckpts_dir.join(format!("{}.seal", req.pre_act_seal_sha256)))?;
    let (_, vc_summary) = verify_checkpoint_causal(&save_bytes_for_vc, &seal_bytes_for_vc)?;
    let build_id = if CRATE_GIT_SHA.len() == 40 {
        CRATE_GIT_SHA
    } else {
        TEST_ENGINE_COMMIT
    };
    let vc_manifest = VerifiedCheckpointsManifest {
        schema_version: VERIFIED_CHECKPOINTS_SCHEMA_VERSION,
        engine_build_id: build_id.to_owned(),
        save_format_version: EVAL_SAVE_FORMAT_VERSION,
        seal_format_version: EVAL_SEAL_FORMAT_VERSION,
        entries: vec![VerifiedCheckpointEntry {
            pre_act_pair_sha256: vc_summary.pre_act_pair_sha256,
            pre_act_save_sha256: vc_summary.pre_act_save_sha256,
            pre_act_seal_sha256: vc_summary.pre_act_seal_sha256,
            engine_commit: vc_summary.engine_commit,
            turn: vc_summary.turn,
            seat_count: vc_summary.seat_count,
            expected_state_digest: vc_summary.expected_state_digest,
            turn_chain_digest: vc_summary.turn_chain_digest,
            post_probe_state_digest: vc_summary.post_probe_state_digest,
        }],
    };
    let vc_path = tmp.path().join("verified_checkpoints.json");
    let vc_text = to_sorted_pretty_json(&vc_manifest)?;
    fs::write(&vc_path, vc_text.as_bytes())?;
    let real_vc_sha = sha256_hex(vc_text.as_bytes());
    let valid_freeze_path = tmp.path().join("valid_freeze.json");
    let valid_freeze_json = serde_json::json!({
        "blind_inputs_manifest_sha256": real_manifest_sha,
        "control_wording_arm": "v2_control_2026_11_22_r2",
        "scorer_config_sha256": cfg_sha,
        "scorer_tree_sha256": real_tree_sha,
        "verified_checkpoints_sha256": real_vc_sha,
    });
    fs::write(
        &valid_freeze_path,
        to_sorted_pretty_json(&valid_freeze_json)?,
    )?;
    let freeze_ok_out = tmp.path().join("out_freeze_ok");
    let mut err_freeze_ok = Vec::new();
    let mut out_freeze_ok_msg = Vec::new();
    assert_eq!(
        league_scorer::cli::run_cli_with_build_id(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--freeze",
                valid_freeze_path.to_str().ok_or("utf8")?,
                "--verified-checkpoints",
                vc_path.to_str().ok_or("utf8")?,
                "--out",
                freeze_ok_out.to_str().ok_or("utf8")?,
            ],
            build_id,
            &mut out_freeze_ok_msg,
            &mut err_freeze_ok,
        ),
        EXIT_OK
    );

    let bad_cfg_freeze = ScorerFreezeBlindView {
        blind_inputs_manifest_sha256: real_manifest_sha.clone(),
        scorer_config_sha256: "f".repeat(64),
        scorer_tree_sha256: real_tree_sha.clone(),
        verified_checkpoints_sha256: real_vc_sha.clone(),
    };
    let bad_tree_freeze = ScorerFreezeBlindView {
        blind_inputs_manifest_sha256: real_manifest_sha.clone(),
        scorer_config_sha256: cfg_sha.clone(),
        scorer_tree_sha256: "e".repeat(64),
        verified_checkpoints_sha256: real_vc_sha.clone(),
    };
    let bad_manifest_freeze = ScorerFreezeBlindView {
        blind_inputs_manifest_sha256: "d".repeat(64),
        scorer_config_sha256: cfg_sha.clone(),
        scorer_tree_sha256: real_tree_sha,
        verified_checkpoints_sha256: real_vc_sha,
    };
    for (idx, (freeze_view, expected_code)) in [
        (bad_cfg_freeze, "FREEZE_CONFIG_MISMATCH"),
        (bad_tree_freeze, "FREEZE_TREE_MISMATCH"),
        (bad_manifest_freeze, "FREEZE_MANIFEST_MISMATCH"),
    ]
    .into_iter()
    .enumerate()
    {
        let p = tmp.path().join(format!("bad_freeze_{idx}.json"));
        fs::write(&p, to_sorted_pretty_json(&freeze_view)?)?;
        let mut err_buf = Vec::new();
        assert_eq!(
            league_scorer::cli::run_cli_with_build_id(
                [
                    "score",
                    "--blind-inputs",
                    blind_dir.to_str().ok_or("utf8")?,
                    "--checkpoints",
                    ckpts_dir.to_str().ok_or("utf8")?,
                    "--config",
                    cfg_path.to_str().ok_or("utf8")?,
                    "--freeze",
                    p.to_str().ok_or("utf8")?,
                    "--verified-checkpoints",
                    vc_path.to_str().ok_or("utf8")?,
                    "--out",
                    tmp.path()
                        .join(format!("out_bad_freeze_{idx}"))
                        .to_str()
                        .ok_or("utf8")?,
                ],
                build_id,
                &mut Vec::new(),
                &mut err_buf,
            ),
            EXIT_USER_ERROR
        );
        assert!(
            String::from_utf8_lossy(&err_buf).contains(expected_code),
            "expected {expected_code}, got {}",
            String::from_utf8_lossy(&err_buf)
        );
    }

    // Also run binary with `--help` and `--version` so CLI help/version paths work end-to-end.
    let bin = env!("CARGO_BIN_EXE_league-scorer");
    for arg in ["--help", "--version"] {
        let out = Command::new(bin).arg(arg).output()?;
        assert!(out.status.success(), "bin {arg}");
        assert!(!out.stdout.is_empty(), "bin {arg}");
    }
    Ok(())
}

#[test]
fn arm_independent_generators_respect_legality_budget_fill_order_and_dedup() -> TestResult {
    let tmp = UniqueTmpDir::new("s2_generators")?;
    let ckpts_dir = tmp.path().join("checkpoints");
    fs::create_dir_all(&ckpts_dir)?;

    let mut host = EvalHost::from_standard_setup(TableId(8201), 4, 1, 17)?
        .with_engine_commit(TEST_ENGINE_COMMIT)?;
    // Advance to turn 2 with ClubBot turns so order books, production inputs, and refined units exist.
    for _ in 0..2 {
        let mut subs = BTreeMap::new();
        for s in 0..4_u8 {
            let obs = host.observe_seat(Seat(s))?;
            let mandate = *obs.own_mandate().ok_or("missing own_mandate")?;
            let dec =
                ClubBot::new(100_u64.wrapping_add(u64::from(s))).decide_full_turn(&obs, &mandate);
            subs.insert(
                s,
                EvalActSubmission {
                    market_actions: dec.pass_a_actions,
                    pass_b_actions: dec.pass_b_actions,
                    bundle: dec.bundle,
                },
            );
        }
        host.apply_act_submissions(&subs)?;
        host.sweep_turn()?;
    }

    // Have peer seat 2 post a wide resting buy/sell book on day 3 before saving the checkpoint.
    let (raw_save_d3, raw_seal_d3) = host.write_pre_act_checkpoint(&tmp.path().join("raw_d3"))?;
    let save_bytes_d3 = fs::read(&raw_save_d3)?;
    let seal_bytes_d3 = fs::read(&raw_seal_d3)?;
    let save_sha_d3 = sha256_hex(&save_bytes_d3);
    let seal_sha_d3 = sha256_hex(&seal_bytes_d3);
    let cas_save_d3 = ckpts_dir.join(format!("{save_sha_d3}.save.json"));
    let cas_seal_d3 = ckpts_dir.join(format!("{seal_sha_d3}.seal"));
    fs::write(&cas_save_d3, &save_bytes_d3)?;
    fs::write(&cas_seal_d3, &seal_bytes_d3)?;

    let obs0 = host.observe_seat(Seat(0))?;
    let mandate0 = *obs0.own_mandate().ok_or("missing focal own_mandate")?;
    let cfg = ScorerConfig::default();

    let cands = generate_arm_independent_candidates(&obs0, &mandate0, &cfg, 55)?;
    let passive_id = candidate_id_for_submission(&EvalActSubmission::default())?;
    let cb_dec = ClubBot::new(55).decide_full_turn(&obs0, &mandate0);
    let cb_sub = EvalActSubmission {
        market_actions: cb_dec.pass_a_actions.clone(),
        pass_b_actions: cb_dec.pass_b_actions.clone(),
        bundle: cb_dec.bundle.clone(),
    };
    let cb_id = candidate_id_for_submission(&cb_sub)?;
    let mut cleared_sub = cb_sub.clone();
    cleared_sub.pass_b_actions.clear();
    let cleared_id = candidate_id_for_submission(&cleared_sub)?;

    assert!(cands.iter().any(|c| c.candidate_id == passive_id));
    assert!(cands.iter().any(|c| c.candidate_id == cb_id));
    if !cb_dec.pass_b_actions.is_empty() {
        assert!(cands.iter().any(|c| c.candidate_id == cleared_id));
    }

    // Sorted strictly ascending by candidate_id, flagged arm-independent + in-span, and all legal.
    let pub_state = obs0.to_public_preview_state();
    for i in 0..cands.len() {
        assert!(cands[i].is_arm_independent);
        assert!(cands[i].in_span);
        assert_eq!(
            cands[i].candidate_id,
            candidate_id_for_submission(&cands[i].submission)?
        );
        if i > 0 {
            assert!(cands[i - 1].candidate_id < cands[i].candidate_id);
        }
        let preview = preview_seat_bundle(
            &pub_state,
            &AiPreviewRequest {
                seat: 0,
                market_actions: cands[i].submission.market_actions.clone(),
                bundle: cands[i].submission.bundle.clone(),
            },
        )?;
        assert!(
            preview.refused_market_actions.is_empty(),
            "synthetic candidate {i} had refused actions: {:?}",
            preview.refused_market_actions
        );
    }

    // Budget cap + fill order sensitivity check when k_synth_max is tight.
    let mut tight_cfg = cfg.clone();
    tight_cfg.k_synth_max = 2;
    let tight_a = generate_arm_independent_candidates(&obs0, &mandate0, &tight_cfg, 55)?;
    assert!(tight_a.len() <= 4); // at most passive + club_bot + 2 synthetics
    assert!(tight_a.iter().any(|c| c.candidate_id == passive_id));
    assert!(tight_a.iter().any(|c| c.candidate_id == cb_id));

    let mut tight_rev = tight_cfg.clone();
    tight_rev.synth_slot_fill_order.reverse();
    let tight_b = generate_arm_independent_candidates(&obs0, &mandate0, &tight_rev, 55)?;
    assert!(tight_b.len() <= 4);
    assert!(tight_b.iter().any(|c| c.candidate_id == passive_id));
    assert!(tight_b.iter().any(|c| c.candidate_id == cb_id));

    // Toggle include_pass_b_cleared_club_bot=false and verify it compiles/runs cleanly without panic.
    let mut no_clear_cfg = cfg.clone();
    no_clear_cfg.include_pass_b_cleared_club_bot = false;
    let no_clear = generate_arm_independent_candidates(&obs0, &mandate0, &no_clear_cfg, 55)?;
    assert!(no_clear.iter().any(|c| c.candidate_id == passive_id));
    assert!(no_clear.iter().any(|c| c.candidate_id == cb_id));

    // dedupe_candidates merges flags using logical OR regardless of input ordering.
    let merged = dedupe_candidates(vec![
        BlindCandidate {
            candidate_id: "0".repeat(64), // overwritten with canonical digest inside dedupe
            in_span: false,
            is_arm_independent: false,
            submission: EvalActSubmission::default(),
        },
        BlindCandidate {
            candidate_id: passive_id.clone(),
            in_span: true,
            is_arm_independent: false,
            submission: EvalActSubmission::default(),
        },
        BlindCandidate {
            candidate_id: passive_id.clone(),
            in_span: false,
            is_arm_independent: true,
            submission: EvalActSubmission::default(),
        },
    ])?;
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].candidate_id, passive_id);
    assert!(merged[0].in_span);
    assert!(merged[0].is_arm_independent);

    // CLI `candidates` command determinism, schema, and sensitivity to checkpoint state.
    let cfg_path = tmp.path().join("scorer_cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;
    let req_out_a = tmp.path().join("cand_out_a");
    let req_out_b = tmp.path().join("cand_out_b");
    let state_id_d3 = "game8201-day-03-seat-0";

    for out_dir in [&req_out_a, &req_out_b] {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let rc = run_cli(
            [
                "candidates",
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--save-sha256",
                &save_sha_d3,
                "--seal-sha256",
                &seal_sha_d3,
                "--focal-seat",
                "0",
                "--state-id",
                state_id_d3,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--scorer-seed",
                "55",
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut stdout,
            &mut stderr,
        );
        assert_eq!(rc, EXIT_OK, "stderr: {}", String::from_utf8_lossy(&stderr));
        assert!(String::from_utf8_lossy(&stdout).contains("candidates="));
    }
    let req_file_a = req_out_a.join(format!("{state_id_d3}.json"));
    let req_file_b = req_out_b.join(format!("{state_id_d3}.json"));
    let bytes_a = fs::read(&req_file_a)?;
    let bytes_b = fs::read(&req_file_b)?;
    assert_eq!(bytes_a, bytes_b, "candidates CLI must be byte-identical");
    let req_text = String::from_utf8(bytes_a)?;
    eprintln!("EXAMPLE_CANDIDATES_REQUEST_JSON:\n{req_text}");
    let parsed_req = parse_blind_scoring_request_strict(req_text.as_bytes())?;
    assert!(parsed_req.talk_commitments.is_empty());
    assert!(parsed_req.validate(&cfg).is_ok());

    // Sensitivity check against a different checkpoint (day 1 fresh table).
    let mut fresh_host = EvalHost::from_standard_setup(TableId(8202), 4, 1, 99)?
        .with_engine_commit(TEST_ENGINE_COMMIT)?;
    let (raw_s1, raw_seal1) = fresh_host.write_pre_act_checkpoint(&tmp.path().join("raw_d1"))?;
    let b_s1 = fs::read(&raw_s1)?;
    let b_seal1 = fs::read(&raw_seal1)?;
    let sha_s1 = sha256_hex(&b_s1);
    let sha_seal1 = sha256_hex(&b_seal1);
    let cas_s1 = ckpts_dir.join(format!("{sha_s1}.save.json"));
    let cas_seal1 = ckpts_dir.join(format!("{sha_seal1}.seal"));
    fs::write(&cas_s1, &b_s1)?;
    fs::write(&cas_seal1, &b_seal1)?;
    let req_d1 = build_arm_independent_request(
        &cas_s1,
        &cas_seal1,
        &sha_s1,
        &sha_seal1,
        0,
        "game8202-day-01-seat-0",
        &cfg,
        55,
    )?;
    assert_ne!(req_d1.candidates, parsed_req.candidates);

    // Feed generated request straight into `score` CLI with ScorerConfig::default() postures!
    let fast_cfg = ScorerConfig {
        k_synth_max: 2,
        scenarios_m: 4,
        ..cfg
    };
    let fast_cfg_path = tmp.path().join("fast_cfg.json");
    fs::write(&fast_cfg_path, to_sorted_pretty_json(&fast_cfg)?)?;
    let fast_req = build_arm_independent_request(
        &cas_s1,
        &cas_seal1,
        &sha_s1,
        &sha_seal1,
        0,
        "game8202-day-01-seat-0",
        &fast_cfg,
        55,
    )?;
    let pipe_in = tmp.path().join("pipe_in");
    let pipe_out = tmp.path().join("pipe_out");
    fs::create_dir_all(&pipe_in)?;
    fs::write(
        pipe_in.join(format!("{}.json", fast_req.state_id)),
        to_sorted_compact_json(&fast_req)?,
    )?;
    let mut score_err = Vec::new();
    let score_rc = run_cli(
        [
            "score",
            "--blind-inputs",
            pipe_in.to_str().ok_or("utf8")?,
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--config",
            fast_cfg_path.to_str().ok_or("utf8")?,
            "--out",
            pipe_out.to_str().ok_or("utf8")?,
        ],
        &mut Vec::new(),
        &mut score_err,
    );
    assert_eq!(
        score_rc,
        EXIT_OK,
        "stderr: {}",
        String::from_utf8_lossy(&score_err)
    );
    Ok(())
}

#[test]
fn postures_schedule_active_liquidity_and_conservative_reserves() -> TestResult {
    let cfg = ScorerConfig::default();
    let expected_schedule = [
        PostureKind::Standard,
        PostureKind::Standard,
        PostureKind::Active,
        PostureKind::Conservative,
    ];
    for m in 0..12_usize {
        assert_eq!(posture_for_scenario(&cfg, m), expected_schedule[m % 4]);
    }

    // Construct a real Turn-2 checkpoint where seat 2 has resting wide spread on Raw(0) and tight on Raw(1).
    let tmp = UniqueTmpDir::new("s2_postures")?;
    let mut host = EvalHost::from_standard_setup(TableId(8210), 4, 1, 21)?
        .with_engine_commit(TEST_ENGINE_COMMIT)?;
    // Sweep Turn 1 so each seat receives starting endowment/yield, then construct Turn 2 book:
    // wide spread on `wide_g` (Seat 1 excess > reserve) and tight spread on `tight_g`.
    let mut turn1_subs = BTreeMap::new();
    for s in 0..4_u8 {
        let mut actions = Vec::new();
        if s == 1 {
            // Import spare Raw(0) for Seat 1 so `Active` definitely has sellable inventory.
            actions.push(MarketAction::TownImport {
                seat: 1,
                good: AssetId::Raw(0),
                amount: 1000,
            });
        }
        if s == 3 {
            actions.push(MarketAction::TownImport {
                seat: 3,
                good: AssetId::Raw(3),
                amount: 500,
            });
        }
        turn1_subs.insert(
            s,
            EvalActSubmission {
                market_actions: actions,
                pass_b_actions: Vec::new(),
                bundle: SealedOrderBundle::default(),
            },
        );
    }
    host.apply_act_submissions(&turn1_subs)?;
    host.sweep_turn()?;

    let obs_probe1 = host.observe_seat(Seat(1))?;
    let obs_probe3 = host.observe_seat(Seat(3))?;
    let tight_g = (0..4_u8)
        .map(AssetId::Raw)
        .find(
            |&g| matches!(g, AssetId::Raw(idx) if obs_probe3.seats[3].raw[usize::from(idx)] >= 100),
        )
        .ok_or("seat 3 lacks >= 100 raw units after Turn 1")?;
    let wide_g = (0..4_u8)
        .map(AssetId::Raw)
        .find(|&g| {
            g != tight_g
                && matches!(g, AssetId::Raw(idx) if obs_probe1.seats[1].raw[usize::from(idx)] >= 200)
        })
        .ok_or("seat 1 lacks spare wide_g raw units after Turn 1")?;

    let mut turn2_subs = BTreeMap::new();
    for s in 0..4_u8 {
        let obs = host.observe_seat(Seat(s))?;
        let parity_wide = obs.parities.asset_parity(wide_g);
        let parity_tight = obs.parities.asset_parity(tight_g);
        let mut actions = Vec::new();
        if s == 2 {
            // Wide spread on `wide_g`: bid at 80% parity (no ask -> spread > 500 bps).
            actions.push(MarketAction::PostOffer {
                seat: 2,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: wide_g,
                    amount: 200,
                    price_per_unit_cred: (parity_wide * 80) / 100,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
            // Tight spread on `tight_g`: bid 99% parity, paired with seat 3 ask at 101% -> spread ~200 bps <= 500 bps.
            actions.push(MarketAction::PostOffer {
                seat: 2,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: tight_g,
                    amount: 100,
                    price_per_unit_cred: (parity_tight * 99) / 100,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
        }
        if s == 3 {
            actions.push(MarketAction::PostOffer {
                seat: 3,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good: tight_g,
                    amount: 100,
                    price_per_unit_cred: (parity_tight * 101) / 100,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
        }
        turn2_subs.insert(
            s,
            EvalActSubmission {
                market_actions: actions,
                pass_b_actions: Vec::new(),
                bundle: SealedOrderBundle::default(),
            },
        );
    }
    host.apply_act_submissions(&turn2_subs)?;
    host.sweep_turn()?;

    let (raw_save, raw_seal) = host.write_pre_act_checkpoint(tmp.path())?;
    let save_bytes = fs::read(&raw_save)?;
    let seal_bytes = fs::read(&raw_seal)?;
    let save_sha = sha256_hex(&save_bytes);
    let seal_sha = sha256_hex(&seal_bytes);
    let pair_sha = compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes);

    let peer_obs = host.observe_seat(Seat(1))?;
    assert!(
        peer_obs
            .offers
            .values()
            .any(|o| o.good == tight_g && o.side == OfferSide::Buy && o.maker != 1),
        "expected tight_g resting buy offer in Turn-2 book"
    );
    assert!(
        peer_obs
            .offers
            .values()
            .any(|o| o.good == tight_g && o.side == OfferSide::Sell && o.maker != 1),
        "expected tight_g resting sell offer in Turn-2 book"
    );
    let peer_mandate = *peer_obs.own_mandate().ok_or("missing peer mandate")?;

    let std_sub = decide_peer_turn(PostureKind::Standard, 700, &peer_obs, &peer_mandate, &cfg)?;
    let act_sub = decide_peer_turn(PostureKind::Active, 700, &peer_obs, &peer_mandate, &cfg)?;
    let cons_sub = decide_peer_turn(
        PostureKind::Conservative,
        700,
        &peer_obs,
        &peer_mandate,
        &cfg,
    )?;

    // Every posture output must pass preview_seat_bundle with zero refused actions.
    let pub_state = peer_obs.to_public_preview_state();
    for sub in [&std_sub, &act_sub, &cons_sub] {
        let pv = preview_seat_bundle(
            &pub_state,
            &AiPreviewRequest {
                seat: 1,
                market_actions: sub.market_actions.clone(),
                bundle: sub.bundle.clone(),
            },
        )?;
        assert!(pv.refused_market_actions.is_empty());
    }

    // Active never adds crossing offers on tight-spread `tight_g` relative to Standard.
    let std_tight_sells = std_sub
        .market_actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                MarketAction::PostOffer { order, .. }
                    if order.side == OfferSide::Sell && order.good == tight_g
            )
        })
        .count();
    let act_tight_sells = act_sub
        .market_actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                MarketAction::PostOffer { order, .. }
                    if order.side == OfferSide::Sell && order.good == tight_g
            )
        })
        .count();
    assert!(
        act_sub.market_actions.len() >= std_sub.market_actions.len(),
        "Active only adds legal crossing PostOffers on wide-spread books"
    );
    assert_eq!(
        act_tight_sells, std_tight_sells,
        "Active must not add extra crossing Sell on tight-spread good"
    );

    // Conservative with very high upkeep_cover_turns drops non-upkeep Buy PostOffer / excess debt repayment.
    let mut strict_cfg = cfg.clone();
    strict_cfg.upkeep_cover_turns = 200;
    let strict_cons = decide_peer_turn(
        PostureKind::Conservative,
        700,
        &peer_obs,
        &peer_mandate,
        &strict_cfg,
    )?;
    let seat_cfg = &peer_obs.spec.seats[1];
    let upkeep_good = AssetId::Refined(seat_cfg.upkeep_good);
    for act in &strict_cons.market_actions {
        if let MarketAction::PostOffer { order, .. } = act
            && order.side == OfferSide::Buy
        {
            assert_eq!(
                order.good, upkeep_good,
                "Conservative with depleted excess CRED only bids for upkeep good"
            );
        }
    }

    // Full rollout under each PostureKind completes identically across repeated calls.
    let focal_obs = host.observe_seat(Seat(0))?;
    let focal_mandate = *focal_obs.own_mandate().ok_or("missing focal mandate")?;
    let scenarios = draw_scenarios(
        &focal_obs.spec,
        focal_mandate,
        0,
        42,
        &pair_sha,
        host.turn(),
        2,
    )?;
    let mut peer_day_subs = BTreeMap::new();
    for s in 1..4_u8 {
        peer_day_subs.insert(s, EvalActSubmission::default());
    }
    let digests = (save_sha.as_str(), seal_sha.as_str(), pair_sha.as_str());
    for posture in [
        PostureKind::Standard,
        PostureKind::Active,
        PostureKind::Conservative,
    ] {
        let out_1 = run_single_rollout(
            &raw_save,
            &raw_seal,
            digests,
            0,
            &EvalActSubmission::default(),
            &peer_day_subs,
            focal_mandate,
            &scenarios[0],
            posture,
            &cfg,
        )?;
        let out_2 = run_single_rollout(
            &raw_save,
            &raw_seal,
            digests,
            0,
            &EvalActSubmission::default(),
            &peer_day_subs,
            focal_mandate,
            &scenarios[0],
            posture,
            &cfg,
        )?;
        assert_eq!(out_1, out_2);
    }
    Ok(())
}

#[test]
fn talk_commitments_canonicalization_bernoulli_rate_and_pruning() -> TestResult {
    let pair_hex = sha256_hex(b"pair");
    let c_legal = ExtractedTalkCommitment {
        counterparty_seat: 2,
        offered_action: MarketAction::TownImport {
            good: AssetId::Raw(0),
            seat: 1,
            amount: 50,
        },
        proposer_seat: 1,
    };
    let c_unaffordable = ExtractedTalkCommitment {
        counterparty_seat: 3,
        offered_action: MarketAction::TownImport {
            good: AssetId::Raw(0),
            seat: 1,
            amount: 1_000_000_000,
        },
        proposer_seat: 1,
    };
    let canon_bytes = to_sorted_compact_json(&c_legal)?;

    // Exact boundary probabilities 0.0, 1.0, and integer conversion validation.
    assert_eq!(p_honour_to_ppm(0.0)?, 0);
    assert_eq!(p_honour_to_ppm(1.0)?, HONOUR_PPM_SCALE);
    assert_eq!(p_honour_to_ppm(0.75)?, 750_000);
    assert_eq!(p_honour_to_ppm(0.25)?, 250_000);
    for invalid_p in [f64::NAN, -0.01, 1.01, f64::INFINITY] {
        assert_eq!(
            p_honour_to_ppm(invalid_p).err().map(|e| e.code()),
            Some("INVALID_SCORER_CONFIG")
        );
    }
    for m in 0..100_usize {
        assert!(!draw_honour_commitment(9, &pair_hex, 1, m, &canon_bytes, 0));
        assert!(draw_honour_commitment(
            9,
            &pair_hex,
            1,
            m,
            &canon_bytes,
            HONOUR_PPM_SCALE
        ));
    }

    // Empirical Bernoulli mean over N=2000 draws within tolerance for p=0.75 and p=0.25.
    for target_p in [0.75_f64, 0.25_f64] {
        let draws = 2000_usize;
        let threshold_ppm = p_honour_to_ppm(target_p)?;
        let count = (0..draws)
            .filter(|&m| draw_honour_commitment(9, &pair_hex, 1, m, &canon_bytes, threshold_ppm))
            .count();
        let mean = (count as f64) / (draws as f64);
        assert!(
            (mean - target_p).abs() <= 0.035,
            "target_p={target_p} empirical mean={mean}"
        );
    }

    // Canonical sort + dedup order-invariance.
    let ab =
        canonicalize_talk_commitments(&[c_legal.clone(), c_unaffordable.clone(), c_legal.clone()])?;
    let ba = canonicalize_talk_commitments(&[c_unaffordable.clone(), c_legal.clone()])?;
    assert_eq!(ab, ba);
    assert_eq!(ab.len(), 2);

    // apply_talk_commitments prepends legal commitment and prunes unaffordable one cleanly.
    let host = EvalHost::from_standard_setup(TableId(8220), 4, 1, 5)?;
    let obs1 = host.observe_seat(Seat(1))?;
    let mandate1 = *obs1.own_mandate().ok_or("missing peer mandate")?;
    let base_sub = decide_peer_turn(
        PostureKind::Standard,
        77,
        &obs1,
        &mandate1,
        &ScorerConfig::default(),
    )?;
    let orig_len = base_sub.market_actions.len();
    let applied = apply_talk_commitments(&obs1, base_sub, &ab, 9, &pair_hex, 0, 1.0)?;
    assert!(applied.market_actions.contains(&c_legal.offered_action));
    assert!(
        !applied
            .market_actions
            .contains(&c_unaffordable.offered_action)
    );
    assert!(applied.market_actions.len() > orig_len);
    let pv = preview_seat_bundle(
        &obs1.to_public_preview_state(),
        &AiPreviewRequest {
            seat: 1,
            market_actions: applied.market_actions,
            bundle: applied.bundle,
        },
    )?;
    assert!(pv.refused_market_actions.is_empty());

    // Validate every invalid and valid counterparty-matched MarketAction variant.
    let dummy_hash = CommitmentHash([0x11; 32]);
    let mut bad_cases = vec![
        ExtractedTalkCommitment {
            proposer_seat: 4,
            counterparty_seat: 1,
            offered_action: MarketAction::TownImport {
                good: AssetId::Raw(0),
                seat: 4,
                amount: 10,
            },
        },
        ExtractedTalkCommitment {
            proposer_seat: 1,
            counterparty_seat: 4,
            offered_action: MarketAction::TownImport {
                good: AssetId::Raw(0),
                seat: 1,
                amount: 10,
            },
        },
        ExtractedTalkCommitment {
            proposer_seat: 1,
            counterparty_seat: 1,
            offered_action: MarketAction::TownImport {
                good: AssetId::Raw(0),
                seat: 1,
                amount: 10,
            },
        },
        ExtractedTalkCommitment {
            proposer_seat: 0,
            counterparty_seat: 1,
            offered_action: MarketAction::TownImport {
                good: AssetId::Raw(0),
                seat: 0,
                amount: 10,
            },
        },
        ExtractedTalkCommitment {
            proposer_seat: 1,
            counterparty_seat: 2,
            offered_action: MarketAction::TownImport {
                good: AssetId::Raw(0),
                seat: 2,
                amount: 10,
            },
        },
        ExtractedTalkCommitment {
            proposer_seat: 1,
            counterparty_seat: 2,
            offered_action: MarketAction::CreateCheck {
                seat: 1,
                check: CheckProposal {
                    payee: 3,
                    asset: AssetId::Cred,
                    send_max: 100,
                    deliver_min: 100,
                    due_turn: 2,
                    expiration_turn: 4,
                },
            },
        },
        ExtractedTalkCommitment {
            proposer_seat: 1,
            counterparty_seat: 2,
            offered_action: MarketAction::CreateEscrow {
                seat: 1,
                escrow: EscrowProposal {
                    destination: 3,
                    asset: AssetId::Cred,
                    amount: 100,
                    finish_after_turn: Some(2),
                    cancel_after_turn: 4,
                    condition_hash: Some(dummy_hash),
                    bond: None,
                },
            },
        },
        ExtractedTalkCommitment {
            proposer_seat: 1,
            counterparty_seat: 2,
            offered_action: MarketAction::FlagWashFreeze {
                inspector: 1,
                target_seat: 3,
            },
        },
        ExtractedTalkCommitment {
            proposer_seat: 1,
            counterparty_seat: 2,
            offered_action: MarketAction::RecordIntentPledge {
                seat: 1,
                pledge: IntentPledge {
                    pledge_id: 1,
                    promisor: 1,
                    counterparty: 3,
                    target: IntentTarget::VoteForSeat {
                        office_turn: 4,
                        candidate: 1,
                    },
                    honor_weight: 50,
                    memo_digest: [0x22; 32],
                },
            },
        },
    ];
    for bad in bad_cases.drain(..) {
        assert_eq!(
            validate_talk_commitment(&bad, 0, 4).err().map(|e| e.code()),
            Some("INVALID_TALK_COMMITMENT")
        );
    }

    for good in [
        ExtractedTalkCommitment {
            proposer_seat: 1,
            counterparty_seat: 2,
            offered_action: MarketAction::CreateCheck {
                seat: 1,
                check: CheckProposal {
                    payee: 2,
                    asset: AssetId::Cred,
                    send_max: 100,
                    deliver_min: 100,
                    due_turn: 2,
                    expiration_turn: 4,
                },
            },
        },
        ExtractedTalkCommitment {
            proposer_seat: 1,
            counterparty_seat: 2,
            offered_action: MarketAction::CreateEscrow {
                seat: 1,
                escrow: EscrowProposal {
                    destination: 2,
                    asset: AssetId::Cred,
                    amount: 100,
                    finish_after_turn: Some(2),
                    cancel_after_turn: 4,
                    condition_hash: Some(dummy_hash),
                    bond: None,
                },
            },
        },
        ExtractedTalkCommitment {
            proposer_seat: 1,
            counterparty_seat: 2,
            offered_action: MarketAction::FlagWashFreeze {
                inspector: 1,
                target_seat: 2,
            },
        },
        ExtractedTalkCommitment {
            proposer_seat: 1,
            counterparty_seat: 2,
            offered_action: MarketAction::RecordIntentPledge {
                seat: 1,
                pledge: IntentPledge {
                    pledge_id: 1,
                    promisor: 1,
                    counterparty: 2,
                    target: IntentTarget::VoteForSeat {
                        office_turn: 4,
                        candidate: 1,
                    },
                    honor_weight: 50,
                    memo_digest: [0x22; 32],
                },
            },
        },
    ] {
        assert!(validate_talk_commitment(&good, 0, 4).is_ok());
    }
    Ok(())
}

#[test]
fn request_hardenings_and_end_to_end_determinism() -> TestResult {
    let tmp = UniqueTmpDir::new("s2_hardenings")?;
    let TestCheckpointFixture {
        cas_save,
        cas_seal,
        save_sha,
        seal_sha,
        pair_sha,
        save_bytes: _,
    } = make_real_checkpoint_pair(tmp.path(), 8230, 0)?;

    let cfg = ScorerConfig {
        k_synth_max: 2,
        scenarios_m: 4, // Rotates [Standard, Standard, Active, Conservative].
        ..ScorerConfig::default()
    };
    let cfg_sha = cfg.sha256_hex()?;
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;

    let cands = generate_arm_independent_candidates(
        &EvalHost::fork_from_checkpoint(&cas_save, &cas_seal)?.observe_seat(Seat(0))?,
        &EvalHost::fork_from_checkpoint(&cas_save, &cas_seal)?
            .observe_seat(Seat(0))?
            .own_mandate()
            .copied()
            .ok_or("missing focal own_mandate")?,
        &cfg,
        1234,
    )?;

    let req = BlindScoringRequest {
        candidates: cands,
        focal_seat: 0,
        pre_act_pair_sha256: pair_sha.clone(),
        pre_act_save_sha256: save_sha.clone(),
        pre_act_seal_sha256: seal_sha.clone(),
        schema_version: BLIND_SCORING_SCHEMA_VERSION,
        scorer_config_sha256: cfg_sha.clone(),
        scorer_seed: 1234,
        state_id: "game8230-day-01-seat-0".to_owned(),
        talk_commitments: vec![ExtractedTalkCommitment {
            counterparty_seat: 2,
            offered_action: MarketAction::TownImport {
                good: AssetId::Raw(0),
                seat: 1,
                amount: 25,
            },
            proposer_seat: 1,
        }],
        talk_extraction_counts: TalkExtractionCounts::default(),
    };
    assert!(req.validate(&cfg).is_ok());

    // Unknown extra JSON fields inside candidate.submission and nested bundle are refused.
    let mut bad_val = serde_json::to_value(&req)?;
    bad_val["candidates"][0]["submission"]["unexpected_field"] = serde_json::json!(true);
    let bad_bytes = serde_json::to_vec(&bad_val)?;
    assert_eq!(
        parse_blind_scoring_request_strict(&bad_bytes)
            .err()
            .map(|e| e.code()),
        Some("UNKNOWN_SUBMISSION_FIELD")
    );

    let mut bad_nested = serde_json::to_value(&req)?;
    bad_nested["candidates"][0]["submission"]["bundle"]["extra_nested"] = serde_json::json!(99);
    let bad_blind = tmp.path().join("bad_nested_blind");
    fs::create_dir_all(&bad_blind)?;
    fs::write(
        bad_blind.join(format!("{}.json", req.state_id)),
        serde_json::to_vec(&bad_nested)?,
    )?;
    let mut err_nested = Vec::new();
    assert_eq!(
        run_cli(
            [
                "score",
                "--blind-inputs",
                bad_blind.to_str().ok_or("utf8")?,
                "--checkpoints",
                tmp.path().join("checkpoints").to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                tmp.path().join("out_bad_nested").to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_nested,
        ),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_nested).contains("UNKNOWN_SUBMISSION_FIELD"));

    // Directory paths are stripped from probe_checkpoint_sanitized error messages.
    let secret_dir = tmp.path().join("very_secret_parent_dir_token");
    fs::create_dir_all(&secret_dir)?;
    let missing_save_path = secret_dir.join("missing_target.save.json");
    let sanitize_err = probe_checkpoint_sanitized(&missing_save_path, &cas_seal)
        .err()
        .ok_or("expected missing file error")?;
    let sanitize_msg = sanitize_err.to_string();
    assert!(
        !sanitize_msg.contains("very_secret_parent_dir_token"),
        "dir leaked in error: {sanitize_msg}"
    );
    assert!(
        !sanitize_msg.contains(&tmp.path().display().to_string()),
        "temp root leaked in error: {sanitize_msg}"
    );
    assert!(sanitize_msg.contains("missing_target.save.json"));

    // Re-verifying save/seal/pair hashes catches mid-run tampering after initial probe.
    verify_checkpoint_pair(&cas_save, &cas_seal, &req)?;
    let tampered_dir = tmp.path().join("midrun_tamper");
    fs::create_dir_all(&tampered_dir)?;
    let t_save = tampered_dir.join("t.save.json");
    let t_seal = tampered_dir.join("t.seal");
    fs::copy(&cas_save, &t_save)?;
    fs::copy(&cas_seal, &t_seal)?;
    verify_checkpoint_pair(&t_save, &t_seal, &req)?;
    fs::write(&t_seal, b"tampered mid-run")?;
    let mut dummy_peers = BTreeMap::new();
    for s in 1..4_u8 {
        dummy_peers.insert(s, EvalActSubmission::default());
    }
    let probe = EvalHost::fork_from_checkpoint(&cas_save, &cas_seal)?;
    let f_obs = probe.observe_seat(Seat(0))?;
    let f_mandate = *f_obs.own_mandate().ok_or("missing focal own_mandate")?;
    let scenarios = draw_scenarios(&f_obs.spec, f_mandate, 0, 1234, &pair_sha, 1, 2)?;
    let tamper_err = run_single_rollout(
        &t_save,
        &t_seal,
        (&save_sha, &seal_sha, &pair_sha),
        0,
        &EvalActSubmission::default(),
        &dummy_peers,
        f_mandate,
        &scenarios[0],
        PostureKind::Active,
        &cfg,
    )
    .err()
    .ok_or("expected mid-run CHECKPOINT_HASH_MISMATCH")?;
    assert_eq!(tamper_err.code(), "CHECKPOINT_HASH_MISMATCH");

    // End-to-end S2 CLI two-pass determinism with postures + non-empty talk_commitments.
    let valid_blind = tmp.path().join("valid_s2_blind");
    fs::create_dir_all(&valid_blind)?;
    fs::write(
        valid_blind.join(format!("{}.json", req.state_id)),
        to_sorted_compact_json(&req)?,
    )?;
    let out_1 = tmp.path().join("s2_out_1");
    let out_2 = tmp.path().join("s2_out_2");
    for out_dir in [&out_1, &out_2] {
        let mut stderr = Vec::new();
        let rc = run_cli(
            [
                "score",
                "--blind-inputs",
                valid_blind.to_str().ok_or("utf8")?,
                "--checkpoints",
                tmp.path().join("checkpoints").to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut stderr,
        );
        assert_eq!(rc, EXIT_OK, "stderr: {}", String::from_utf8_lossy(&stderr));
    }
    let b1 = fs::read(out_1.join("scores/game8230-day-01-seat-0.json"))?;
    let b2 = fs::read(out_2.join("scores/game8230-day-01-seat-0.json"))?;
    assert_eq!(
        b1, b2,
        "S2 two-pass CLI score output must be byte-identical"
    );

    // Verify `candidates` CLI usage & digest error branches cleanly.
    for bad_args in [
        vec!["candidates"],
        vec!["candidates", "--unknown"],
        vec!["candidates", "--focal-seat", "notanint"],
        vec!["candidates", "--scorer-seed", "notau64"],
        vec![
            "candidates",
            "--checkpoints",
            tmp.path().join("checkpoints").to_str().ok_or("utf8")?,
            "--save-sha256",
            &save_sha,
            "--seal-sha256",
            &seal_sha,
            "--focal-seat",
            "0",
            "--state-id",
            "game8230-day-01-seat-0",
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--scorer-seed",
            "1",
        ],
    ] {
        let mut err = Vec::new();
        assert_eq!(
            run_cli(bad_args, &mut Vec::new(), &mut err),
            EXIT_USER_ERROR
        );
        assert!(!err.is_empty());
    }

    // Tampered save sha passed to `candidates` CLI returns CHECKPOINT_HASH_MISMATCH without leaking full paths.
    let mut err_sha = Vec::new();
    let bad_sha = "0".repeat(64);
    assert_eq!(
        run_cli(
            [
                "candidates",
                "--checkpoints",
                tmp.path().join("checkpoints").to_str().ok_or("utf8")?,
                "--save-sha256",
                &bad_sha,
                "--seal-sha256",
                &seal_sha,
                "--focal-seat",
                "0",
                "--state-id",
                "game8230-day-01-seat-0",
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--scorer-seed",
                "1",
                "--out",
                tmp.path().join("cand_err_out").to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_sha,
        ),
        EXIT_USER_ERROR
    );
    let err_sha_s = String::from_utf8_lossy(&err_sha);
    assert!(
        err_sha_s.contains("CHECKPOINT_NOT_FOUND")
            || err_sha_s.contains("CHECKPOINT_HASH_MISMATCH")
    );
    Ok(())
}

#[test]
fn coverage_s2_turn4_turn6_books_debt_claims_and_cli_edges() -> TestResult {
    let tmp = UniqueTmpDir::new("s2_cov_edges")?;
    let cfg = ScorerConfig::default();
    let cfg_path = tmp.path().join("cfg.json");
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;

    // Advance a season through Turn 4 (election) and Turn 6 (charter auction) with:
    // - maturing check + unconditioned escrow payable to Seat 0
    // - competing multi-level Buy/Sell resting offers on Raw goods (testing best_buy/best_sell updates)
    // - excess CRED locked/released so Seat 0 accumulates cred_debt and then holds spare CRED
    let mut host = EvalHost::from_standard_setup(TableId(8250), 4, 1, 31)?
        .with_engine_commit(TEST_ENGINE_COMMIT)?;
    let mut day1 = BTreeMap::new();
    for s in 0..4_u8 {
        let obs = host.observe_seat(Seat(s))?;
        let p0 = obs.parities.asset_parity(AssetId::Raw(0));
        let p1 = obs.parities.asset_parity(AssetId::Raw(1));
        let cred = obs.seats[usize::from(s)].cred;
        let mut actions = Vec::new();
        if s == 1 {
            // Peer 1 draws a check payable to Seat 0 maturing on Turn 3, and places a lower Buy bid.
            actions.push(MarketAction::CreateCheck {
                seat: 1,
                check: CheckProposal {
                    payee: 0,
                    asset: AssetId::Cred,
                    send_max: 25,
                    deliver_min: 25,
                    due_turn: 3,
                    expiration_turn: 8,
                },
            });
            actions.push(MarketAction::PostOffer {
                seat: 1,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(0),
                    amount: 100,
                    price_per_unit_cred: (p0 * 70) / 100,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
        }
        if s == 2 {
            // Peer 2 creates an unconditioned escrow to Seat 0 and posts higher + equal Buy bids on Raw(0).
            actions.push(MarketAction::CreateEscrow {
                seat: 2,
                escrow: EscrowProposal {
                    destination: 0,
                    asset: AssetId::Cred,
                    amount: 50,
                    finish_after_turn: Some(3),
                    cancel_after_turn: 10,
                    condition_hash: None,
                    bond: None,
                },
            });
            actions.push(MarketAction::PostOffer {
                seat: 2,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(0),
                    amount: 100,
                    price_per_unit_cred: (p0 * 75) / 100,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
            actions.push(MarketAction::PostOffer {
                seat: 2,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: AssetId::Raw(0),
                    amount: 50,
                    price_per_unit_cred: (p0 * 75) / 100,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
        }
        if s == 3 {
            actions.push(MarketAction::TownImport {
                seat: 3,
                good: AssetId::Raw(1),
                amount: 300,
            });
            let _ = p1;
        }
        if s == 0 && cred > 10_000 {
            // Lock Seat 0's CRED in an unreachable low bid on Raw(2) to force Turn-1 upkeep shortfall debt.
            let bid_price = 100;
            let amt = ((i128::from(cred - 50) * 1_000) / i128::from(bid_price)) as i64;
            if amt > 0 {
                actions.push(MarketAction::PostOffer {
                    seat: 0,
                    order: OfferProposal {
                        side: OfferSide::Buy,
                        good: AssetId::Raw(2),
                        amount: amt,
                        price_per_unit_cred: bid_price,
                        domain_id: None,
                        is_hybrid: false,
                    },
                });
            }
        }
        day1.insert(
            s,
            EvalActSubmission {
                market_actions: actions,
                pass_b_actions: Vec::new(),
                bundle: SealedOrderBundle::default(),
            },
        );
    }
    host.apply_act_submissions(&day1)?;
    host.sweep_turn()?;

    // Turn 2: cancel Seat 0's locking offer so Seat 0 regains free CRED while carrying cred_debt,
    // and let Seat 3 post two Sell offers on Raw(1) at different prices.
    let mut day2 = BTreeMap::new();
    for s in 0..4_u8 {
        let obs = host.observe_seat(Seat(s))?;
        let mut actions = Vec::new();
        if s == 0 {
            for offer in obs.offers.values() {
                if offer.maker == 0 {
                    actions.push(MarketAction::CancelOffer {
                        seat: 0,
                        offer_id: offer.offer_id,
                    });
                }
            }
        }
        if s == 3 {
            let p1 = obs.parities.asset_parity(AssetId::Raw(1));
            actions.push(MarketAction::PostOffer {
                seat: 3,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good: AssetId::Raw(1),
                    amount: 100,
                    price_per_unit_cred: (p1 * 130) / 100,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
            actions.push(MarketAction::PostOffer {
                seat: 3,
                order: OfferProposal {
                    side: OfferSide::Sell,
                    good: AssetId::Raw(1),
                    amount: 100,
                    price_per_unit_cred: (p1 * 120) / 100,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
        }
        day2.insert(
            s,
            EvalActSubmission {
                market_actions: actions,
                pass_b_actions: Vec::new(),
                bundle: SealedOrderBundle::default(),
            },
        );
    }
    host.apply_act_submissions(&day2)?;
    host.sweep_turn()?;

    // Turn 3 -> Turn 4: import 2 batches of project[0] refined inputs for Seat 0 so `gen_batches`
    // covers `project_deliveries`, composite drops, and scaling; keep peers mostly idle to preserve book depth.
    while host.turn() < 4 {
        let mut subs = BTreeMap::new();
        for s in 0..4_u8 {
            let obs = host.observe_seat(Seat(s))?;
            let mandate = *obs.own_mandate().ok_or("missing own_mandate")?;
            let dec =
                ClubBot::new(200_u64.wrapping_add(u64::from(s))).decide_full_turn(&obs, &mandate);
            let market_actions = if s == 0 {
                let mut acts = Vec::new();
                if let Some(proj) = obs.spec.projects.first() {
                    let upkeep_amt = obs.spec.seats[0].upkeep_amount;
                    for &(g_idx, per_batch) in &proj.inputs {
                        if per_batch > 0 {
                            acts.push(MarketAction::TownImport {
                                seat: 0,
                                good: AssetId::Refined(g_idx),
                                amount: per_batch.saturating_mul(2).saturating_add(upkeep_amt),
                            });
                        }
                    }
                }
                acts
            } else if s == 3 {
                let p1 = obs.parities.asset_parity(AssetId::Raw(1));
                vec![
                    MarketAction::PostOffer {
                        seat: 3,
                        order: OfferProposal {
                            side: OfferSide::Sell,
                            good: AssetId::Raw(1),
                            amount: 100,
                            price_per_unit_cred: (p1 * 130) / 100,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    },
                    MarketAction::PostOffer {
                        seat: 3,
                        order: OfferProposal {
                            side: OfferSide::Sell,
                            good: AssetId::Raw(1),
                            amount: 100,
                            price_per_unit_cred: (p1 * 120) / 100,
                            domain_id: None,
                            is_hybrid: false,
                        },
                    },
                ]
            } else if s == 1 || s == 2 {
                Vec::new()
            } else {
                dec.pass_a_actions
            };
            subs.insert(
                s,
                EvalActSubmission {
                    market_actions,
                    pass_b_actions: if s == 0 {
                        Vec::new()
                    } else {
                        dec.pass_b_actions
                    },
                    bundle: SealedOrderBundle::default(),
                },
            );
        }
        host.apply_act_submissions(&subs)?;
        host.sweep_turn()?;
    }

    // Turn 4 (election): generate arm-independent candidates and exercise Active/Conservative on multi-offer books.
    assert_eq!(host.turn(), 4);
    let obs_t4 = host.observe_seat(Seat(0))?;
    let mandate_t4 = *obs_t4.own_mandate().ok_or("missing focal own_mandate")?;
    let cands_t4 = generate_arm_independent_candidates(&obs_t4, &mandate_t4, &cfg, 77)?;
    assert!(!cands_t4.is_empty());
    let pub4 = obs_t4.to_public_preview_state();
    for cand in &cands_t4 {
        let pv = preview_seat_bundle(
            &pub4,
            &AiPreviewRequest {
                seat: 0,
                market_actions: cand.submission.market_actions.clone(),
                bundle: cand.submission.bundle.clone(),
            },
        )?;
        assert!(pv.refused_market_actions.is_empty());
    }
    // Exercise Active & Conservative on Seat 1/3 books on Turn 4.
    for seat_idx in [1_u8, 2, 3] {
        let obs_s = host.observe_seat(Seat(seat_idx))?;
        let m_s = *obs_s.own_mandate().ok_or("missing mandate")?;
        let _ = decide_peer_turn(PostureKind::Active, 700, &obs_s, &m_s, &cfg)?;
        let _ = decide_peer_turn(PostureKind::Conservative, 700, &obs_s, &m_s, &cfg)?;
    }

    // Exercise rich-inventory/debt synthetic generation (`ranked_project_batches`, `gen_debt`, `composite_drops`, `scaled`).
    let mut obs_rich = obs_t4.clone();
    obs_rich.seats[0].raw.fill(10_000);
    obs_rich.seats[0].refined.fill(10_000);
    obs_rich.seats[0].cred = 100_000;
    obs_rich.seats[0].cred_debt = 500;
    let rich_cands = generate_arm_independent_candidates(&obs_rich, &mandate_t4, &cfg, 77)?;
    assert!(
        rich_cands
            .iter()
            .any(|c| !c.submission.bundle.project_deliveries.is_empty()),
        "rich inventory should generate legal project_deliveries synthetics"
    );
    assert!(
        rich_cands
            .iter()
            .any(|c| c.submission.bundle.debt_repayment_cred > 0),
        "rich CRED + debt should generate debt_repayment_cred synthetics"
    );

    // Exercise apply_talk_commitments, cancel_locked, and wash_frozen edge cases.
    let mut obs_locked = obs_t4.clone();
    obs_locked.cancel_locked = true;
    let locked_cands = generate_arm_independent_candidates(&obs_locked, &mandate_t4, &cfg, 77)?;
    assert!(!locked_cands.is_empty());
    let mut obs_frozen = obs_t4.clone();
    obs_frozen.seats[0].wash_frozen_until_turn = obs_frozen.turn;
    let frozen_cands = generate_arm_independent_candidates(&obs_frozen, &mandate_t4, &cfg, 77)?;
    assert!(!frozen_cands.is_empty());
    let _ = decide_peer_turn(PostureKind::Active, 700, &obs_frozen, &mandate_t4, &cfg)?;
    let dummy_sub = EvalActSubmission::default();
    let other_comm = vec![ExtractedTalkCommitment {
        counterparty_seat: 2,
        offered_action: MarketAction::TownImport {
            good: AssetId::Raw(0),
            seat: 1,
            amount: 10,
        },
        proposer_seat: 1,
    }];
    assert_eq!(
        apply_talk_commitments(
            &obs_locked,
            dummy_sub.clone(),
            &other_comm,
            1,
            &"a".repeat(64),
            0,
            1.0,
        )?,
        dummy_sub
    );
    assert_eq!(
        apply_talk_commitments(
            &obs_t4,
            dummy_sub.clone(),
            &other_comm,
            1,
            &"a".repeat(64),
            0,
            0.0,
        )?,
        dummy_sub
    );

    // Advance to Turn 6 (charter auction) and verify candidate generation + CLI binary `candidates` invocation.
    while host.turn() < 6 && host.phase() == TurnPhase::TalkAndMarket {
        let mut subs = BTreeMap::new();
        for s in 0..4_u8 {
            let obs = host.observe_seat(Seat(s))?;
            let mandate = *obs.own_mandate().ok_or("missing own_mandate")?;
            let dec =
                ClubBot::new(300_u64.wrapping_add(u64::from(s))).decide_full_turn(&obs, &mandate);
            subs.insert(
                s,
                EvalActSubmission {
                    market_actions: dec.pass_a_actions,
                    pass_b_actions: dec.pass_b_actions,
                    bundle: dec.bundle,
                },
            );
        }
        host.apply_act_submissions(&subs)?;
        host.sweep_turn()?;
    }
    if host.turn() == 6 && host.phase() == TurnPhase::TalkAndMarket {
        let obs_t6 = host.observe_seat(Seat(0))?;
        let mandate_t6 = *obs_t6.own_mandate().ok_or("missing focal mandate")?;
        let cands_t6 = generate_arm_independent_candidates(&obs_t6, &mandate_t6, &cfg, 88)?;
        assert!(cands_t6.iter().all(|c| c.is_arm_independent && c.in_span));
    }

    // build_arm_independent_request validation error branches (bad hexes, bad focal_seat, 5-seat save).
    let ckpts_dir = tmp.path().join("checkpoints");
    let raw_t6_dir = tmp.path().join("raw_t6");
    fs::create_dir_all(&raw_t6_dir)?;
    let (raw_s, raw_seal) = host.write_pre_act_checkpoint(&raw_t6_dir)?;
    let b_s = fs::read(&raw_s)?;
    let b_seal = fs::read(&raw_seal)?;
    let sha_s = sha256_hex(&b_s);
    let sha_seal = sha256_hex(&b_seal);
    fs::create_dir_all(&ckpts_dir)?;
    let cas_s = ckpts_dir.join(format!("{sha_s}.save.json"));
    let cas_seal = ckpts_dir.join(format!("{sha_seal}.seal"));
    fs::write(&cas_s, &b_s)?;
    fs::write(&cas_seal, &b_seal)?;

    assert_eq!(
        build_arm_independent_request(&cas_s, &cas_seal, "short", &sha_seal, 0, "id1", &cfg, 1)
            .err()
            .map(|e| e.code()),
        Some("INVALID_HEX_DIGEST")
    );
    assert_eq!(
        build_arm_independent_request(&cas_s, &cas_seal, &sha_s, "short", 0, "id1", &cfg, 1)
            .err()
            .map(|e| e.code()),
        Some("INVALID_HEX_DIGEST")
    );
    assert_eq!(
        build_arm_independent_request(&cas_s, &cas_seal, &sha_s, &sha_seal, 4, "id1", &cfg, 1)
            .err()
            .map(|e| e.code()),
        Some("INVALID_FOCAL_SEAT")
    );

    let mut host5 = EvalHost::from_standard_setup(TableId(8251), 5, 1, 0)?
        .with_engine_commit(TEST_ENGINE_COMMIT)?;
    let raw5_dir = tmp.path().join("raw5");
    fs::create_dir_all(&raw5_dir)?;
    let (s5, seal5) = host5.write_pre_act_checkpoint(&raw5_dir)?;
    let sha5 = sha256_hex(&fs::read(&s5)?);
    let sha5_seal = sha256_hex(&fs::read(&seal5)?);
    assert_eq!(
        build_arm_independent_request(&s5, &seal5, &sha5, &sha5_seal, 0, "id5", &cfg, 1)
            .err()
            .map(|e| e.code()),
        Some("INVALID_CHECKPOINT_STATE")
    );

    // parse_blind_scoring_request_strict structural edge errors.
    for bad_json in [
        b"{bad".as_slice(),
        b"{\"candidates\":{},\"focal_seat\":0}".as_slice(),
        b"{\"candidates\":[{\"candidate_id\":\"a\",\"in_span\":true,\"is_arm_independent\":true,\"submission\":{\"bundle\":{},\"market_actions\":[],\"pass_b_actions\":[]}}]}".as_slice(),
    ] {
        assert!(parse_blind_scoring_request_strict(bad_json).is_err());
    }

    // Cover all remaining `candidates` CLI missing/invalid/IO error paths and compiled binary execution.
    for bad_args in [
        vec!["candidates", "--checkpoints", "a", "--checkpoints", "b"],
        vec!["candidates", "--save-sha256", "a", "--save-sha256", "b"],
        vec!["candidates", "--seal-sha256", "a", "--seal-sha256", "b"],
        vec!["candidates", "--focal-seat", "0", "--focal-seat", "1"],
        vec!["candidates", "--state-id", "a", "--state-id", "b"],
        vec!["candidates", "--config", "a", "--config", "b"],
        vec!["candidates", "--scorer-seed", "1", "--scorer-seed", "2"],
        vec!["candidates", "--out", "a", "--out", "b"],
        vec!["candidates", "--error-format", "json"],
        vec!["candidates", "--error-format", "xml"],
        vec!["candidates", "--save-sha256", &sha_s],
        vec![
            "candidates",
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
        ],
        vec![
            "candidates",
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--save-sha256",
            &sha_s,
        ],
        vec![
            "candidates",
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--save-sha256",
            &sha_s,
            "--seal-sha256",
            &sha_seal,
        ],
        vec![
            "candidates",
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--save-sha256",
            &sha_s,
            "--seal-sha256",
            &sha_seal,
            "--focal-seat",
            "0",
        ],
        vec![
            "candidates",
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--save-sha256",
            &sha_s,
            "--seal-sha256",
            &sha_seal,
            "--focal-seat",
            "0",
            "--state-id",
            "game8250-day-06-seat-0",
        ],
        vec![
            "candidates",
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--save-sha256",
            &sha_s,
            "--seal-sha256",
            &sha_seal,
            "--focal-seat",
            "0",
            "--state-id",
            "game8250-day-06-seat-0",
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
        ],
        vec![
            "candidates",
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--save-sha256",
            "badhex",
            "--seal-sha256",
            &sha_seal,
            "--focal-seat",
            "0",
            "--state-id",
            "game8250-day-06-seat-0",
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--scorer-seed",
            "1",
            "--out",
            tmp.path().join("o_bad1").to_str().ok_or("utf8")?,
        ],
        vec![
            "candidates",
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--save-sha256",
            &sha_s,
            "--seal-sha256",
            "badhex",
            "--focal-seat",
            "0",
            "--state-id",
            "game8250-day-06-seat-0",
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--scorer-seed",
            "1",
            "--out",
            tmp.path().join("o_bad2").to_str().ok_or("utf8")?,
        ],
    ] {
        let mut err = Vec::new();
        assert_eq!(
            run_cli(bad_args, &mut Vec::new(), &mut err),
            EXIT_USER_ERROR
        );
        assert!(!err.is_empty());
    }

    // `--out` as file + stdout failure + real binary execution for `candidates`.
    let out_file = tmp.path().join("out_is_regular_file");
    fs::write(&out_file, b"x")?;
    let mut err_file_out = Vec::new();
    assert_eq!(
        run_cli(
            [
                "candidates",
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--save-sha256",
                &sha_s,
                "--seal-sha256",
                &sha_seal,
                "--focal-seat",
                "0",
                "--state-id",
                "game8250-day-06-seat-0",
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--scorer-seed",
                "1",
                "--out",
                out_file.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_file_out,
        ),
        EXIT_RUNTIME_ERROR
    );

    let bin = env!("CARGO_BIN_EXE_league-scorer");
    let bin_out = tmp.path().join("bin_cands_out");
    let proc_cands = Command::new(bin)
        .args([
            "candidates",
            "--checkpoints",
            ckpts_dir.to_str().ok_or("utf8")?,
            "--save-sha256",
            &sha_s,
            "--seal-sha256",
            &sha_seal,
            "--focal-seat",
            "0",
            "--state-id",
            "game8250-day-06-seat-0",
            "--config",
            cfg_path.to_str().ok_or("utf8")?,
            "--scorer-seed",
            "1",
            "--out",
            bin_out.to_str().ok_or("utf8")?,
        ])
        .output()?;
    assert!(
        proc_cands.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&proc_cands.stderr)
    );
    assert!(bin_out.join("game8250-day-06-seat-0.json").is_file());

    // Cover `candidates` CLI stdout write failure, out_path-is-directory write failure, and dir-as-checkpoint errors.
    let mut fail_out_writer = FailingWriter;
    let mut err_cands_stdout = Vec::new();
    assert_eq!(
        run_cli(
            [
                "candidates",
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--save-sha256",
                &sha_s,
                "--seal-sha256",
                &sha_seal,
                "--focal-seat",
                "0",
                "--state-id",
                "game8250-day-06-seat-0",
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--scorer-seed",
                "1",
                "--out",
                tmp.path()
                    .join("cands_stdout_fail")
                    .to_str()
                    .ok_or("utf8")?,
            ],
            &mut fail_out_writer,
            &mut err_cands_stdout,
        ),
        EXIT_RUNTIME_ERROR
    );

    let out_conflict_dir = tmp.path().join("cands_conflict");
    fs::create_dir_all(out_conflict_dir.join("game8250-day-06-seat-0.json"))?;
    let mut err_write_conflict = Vec::new();
    assert_eq!(
        run_cli(
            [
                "candidates",
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--save-sha256",
                &sha_s,
                "--seal-sha256",
                &sha_seal,
                "--focal-seat",
                "0",
                "--state-id",
                "game8250-day-06-seat-0",
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--scorer-seed",
                "1",
                "--out",
                out_conflict_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_write_conflict,
        ),
        EXIT_RUNTIME_ERROR
    );

    let req_dir_err = BlindScoringRequest {
        candidates: vec![build_candidate(EvalActSubmission::default(), true, true)?],
        focal_seat: 0,
        pre_act_pair_sha256: sha256_hex(b"p"),
        pre_act_save_sha256: sha_s.clone(),
        pre_act_seal_sha256: sha_seal.clone(),
        schema_version: 1,
        scorer_config_sha256: cfg.sha256_hex()?,
        scorer_seed: 1,
        state_id: "dir_err".to_owned(),
        talk_commitments: Vec::new(),
        talk_extraction_counts: TalkExtractionCounts::default(),
    };
    let dir_as_file = tmp.path().join("dir_as_ckpt");
    fs::create_dir_all(&dir_as_file)?;
    assert_eq!(
        verify_checkpoint_pair(&dir_as_file, &cas_seal, &req_dir_err)
            .err()
            .map(|e| e.code()),
        Some("IO_ERROR")
    );
    assert_eq!(
        verify_checkpoint_pair(&cas_s, &dir_as_file, &req_dir_err)
            .err()
            .map(|e| e.code()),
        Some("IO_ERROR")
    );
    Ok(())
}

fn extract_backticked_tokens(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else {
            break;
        };
        out.push(&after[..close]);
        rest = &after[close + 1..];
    }
    out
}

fn parse_sec52_table(sec52: &str) -> Result<BTreeMap<String, String>, String> {
    let mut rows = BTreeMap::new();
    let mut in_table = false;
    let mut saw_separator = false;
    for raw_line in sec52.lines() {
        let line = raw_line.trim();
        if line.starts_with("| Proposed Parameter (`ScorerConfig` field)") {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        if !line.starts_with('|') {
            break;
        }
        if line.starts_with("|---") {
            saw_separator = true;
            continue;
        }
        if !saw_separator {
            return Err("expected markdown separator row after header".to_owned());
        }
        let cols: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cols.len() != 3 {
            return Err(format!("expected 3 columns in §5.2 table row `{line}`"));
        }
        let field_tokens: Vec<&str> = extract_backticked_tokens(cols[0])
            .into_iter()
            .filter(|tok| {
                !tok.is_empty()
                    && tok
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            })
            .collect();
        if field_tokens.is_empty() {
            return Err(format!("no field name found in row `{line}`"));
        }
        for field in field_tokens {
            if rows.insert(field.to_owned(), cols[2].to_owned()).is_some() {
                return Err(format!("duplicate field `{field}` in §5.2 table"));
            }
        }
    }
    if rows.is_empty() {
        return Err("failed to parse any §5.2 table rows".to_owned());
    }
    Ok(rows)
}

fn first_backticked(cell: &str, field: &str) -> Result<String, String> {
    extract_backticked_tokens(cell)
        .into_iter()
        .next()
        .map(str::to_owned)
        .ok_or_else(|| format!("missing backticked value in §5.2 cell for `{field}`: `{cell}`"))
}

fn unquote_doc_str(tok: &str, field: &str) -> Result<String, String> {
    tok.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .map(str::to_owned)
        .ok_or_else(|| format!("expected quoted string in §5.2 for `{field}`, got `{tok}`"))
}

fn parse_doc_posture_kind(tok: &str) -> Result<PostureKind, String> {
    match tok {
        "Std" | "Standard" => Ok(PostureKind::Standard),
        "Act" | "Active" => Ok(PostureKind::Active),
        "Cons" | "Conservative" => Ok(PostureKind::Conservative),
        _ => Err(format!("unknown posture token `{tok}` in §5.2 cell")),
    }
}

fn verify_sec52_table_matches_defaults(sec52: &str) -> Result<(), String> {
    let table = parse_sec52_table(sec52)?;
    // Exhaustive destructuring without `..` ensures any added ScorerConfig or PostureWeights
    // field fails to compile until explicitly mapped to its own row in §5.2.
    let ScorerConfig {
        composite_greedy_rules,
        debt_repayment_grid_bps,
        include_pass_b_cleared_club_bot,
        k_synth_max,
        mandate_stratification_mode,
        p_honour,
        parity_spread_threshold_bps,
        partial_crossing_fractions_bps,
        posture_schedule,
        posture_weights:
            PostureWeights {
                active,
                conservative,
                standard,
            },
        prescreen_exclude_flat_sets,
        prescreen_min_arm_indep_candidates,
        prescreen_min_arm_indep_span,
        quantity_scales_bps,
        quote_offsets_bps,
        scenarios_m,
        strata_schedule,
        synth_slot_fill_order,
        talk_extractor_rules_digest,
        talk_extractor_version,
        upkeep_cover_turns,
    } = ScorerConfig::default();

    const EXPECTED_FIELD_COUNT: usize = 21;
    if table.len() != EXPECTED_FIELD_COUNT {
        return Err(format!(
            "§5.2 table mapped {} fields, expected {EXPECTED_FIELD_COUNT}",
            table.len()
        ));
    }
    let get = |name: &str| -> Result<&str, String> {
        table
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("§5.2 table missing field `{name}`"))
    };

    // 1–4. posture_weights.{standard, active, conservative} & posture_schedule
    let pw_cell = get("posture_weights")?;
    let ps_cell = get("posture_schedule")?;
    if pw_cell != ps_cell {
        return Err("posture_weights and posture_schedule cells disagree".to_owned());
    }
    let pw_tokens = extract_backticked_tokens(pw_cell);
    if pw_tokens.len() < 2 {
        return Err(format!("invalid posture cell `{pw_cell}`"));
    }
    let mix_inner = pw_tokens[0]
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| format!("expected bracketed weight mix in `{pw_cell}`"))?;
    let mut doc_std = None;
    let mut doc_act = None;
    let mut doc_cons = None;
    for part in mix_inner.split(',').map(str::trim) {
        let mut it = part.split_whitespace();
        let w: u32 = it
            .next()
            .ok_or_else(|| format!("missing weight count in `{part}`"))?
            .parse()
            .map_err(|e| format!("bad weight `{part}`: {e}"))?;
        let label = it
            .next()
            .ok_or_else(|| format!("missing posture label in `{part}`"))?;
        match label {
            "Standard" => doc_std = Some(w),
            "Active" => doc_act = Some(w),
            "Conservative" => doc_cons = Some(w),
            other => return Err(format!("unknown posture label `{other}`")),
        }
    }
    if (doc_std, doc_act, doc_cons) != (Some(standard), Some(active), Some(conservative)) {
        return Err(format!(
            "posture_weights mismatch: doc=({doc_std:?},{doc_act:?},{doc_cons:?}) default=({standard},{active},{conservative})"
        ));
    }
    let mapping = pw_tokens[1]
        .strip_prefix("m % 4")
        .ok_or_else(|| format!("expected `m % 4` schedule mapping in `{}`", pw_tokens[1]))?
        .trim();
    // Parse schedule robustly on `m % 4` without brace comma ambiguity by splitting on `->`.
    let mut doc_sched_vec = vec![PostureKind::Standard; 4];
    let mut seen_mask = [false; 4];
    let arrow_parts: Vec<&str> = mapping.split("->").map(str::trim).collect();
    if arrow_parts.len() < 2 {
        return Err(format!(
            "missing `->` schedule mapping in `{}`",
            pw_tokens[1]
        ));
    }
    for i in 0..arrow_parts.len() - 1 {
        let idx_side = if i == 0 {
            arrow_parts[i]
        } else {
            arrow_parts[i]
                .split_once(',')
                .map(|(_, rest)| rest.trim())
                .unwrap_or("")
        };
        let rhs_raw = arrow_parts[i + 1];
        let label = if i + 1 == arrow_parts.len() - 1 {
            rhs_raw
        } else {
            rhs_raw
                .split_once(',')
                .map(|(lbl, _)| lbl.trim())
                .ok_or_else(|| format!("malformed mapping rhs `{rhs_raw}`"))?
        };
        let kind = parse_doc_posture_kind(label)?;
        let cleaned = idx_side.trim_start_matches("in ").trim();
        let idxs_str = cleaned
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .unwrap_or(cleaned);
        for num_s in idxs_str.split(',').map(str::trim) {
            let idx: usize = num_s
                .parse()
                .map_err(|e| format!("bad slot index `{num_s}`: {e}"))?;
            if idx >= 4 {
                return Err(format!("slot index {idx} >= 4"));
            }
            doc_sched_vec[idx] = kind;
            seen_mask[idx] = true;
        }
    }
    if !seen_mask.into_iter().all(|b| b) || doc_sched_vec != posture_schedule {
        return Err(format!(
            "posture_schedule mismatch: doc={doc_sched_vec:?} seen={seen_mask:?} default={posture_schedule:?}"
        ));
    }

    // 5. p_honour
    let doc_p: f64 = first_backticked(get("p_honour")?, "p_honour")?
        .parse()
        .map_err(|e| format!("bad p_honour: {e}"))?;
    if doc_p != p_honour {
        return Err(format!("p_honour mismatch: doc={doc_p} default={p_honour}"));
    }

    // 6. scenarios_m
    let doc_m: usize = first_backticked(get("scenarios_m")?, "scenarios_m")?
        .parse()
        .map_err(|e| format!("bad scenarios_m: {e}"))?;
    if doc_m != scenarios_m {
        return Err(format!(
            "scenarios_m mismatch: doc={doc_m} default={scenarios_m}"
        ));
    }

    // 7. k_synth_max
    let doc_k: usize = first_backticked(get("k_synth_max")?, "k_synth_max")?
        .parse()
        .map_err(|e| format!("bad k_synth_max: {e}"))?;
    if doc_k != k_synth_max {
        return Err(format!(
            "k_synth_max mismatch: doc={doc_k} default={k_synth_max}"
        ));
    }

    // 8. include_pass_b_cleared_club_bot
    let doc_inc: bool = first_backticked(
        get("include_pass_b_cleared_club_bot")?,
        "include_pass_b_cleared_club_bot",
    )?
    .parse()
    .map_err(|e| format!("bad include_pass_b_cleared_club_bot: {e}"))?;
    if doc_inc != include_pass_b_cleared_club_bot {
        return Err(format!(
            "include_pass_b_cleared_club_bot mismatch: doc={doc_inc} default={include_pass_b_cleared_club_bot}"
        ));
    }

    // 9. partial_crossing_fractions_bps
    let doc_part: Vec<u16> = serde_json::from_str(&first_backticked(
        get("partial_crossing_fractions_bps")?,
        "partial_crossing_fractions_bps",
    )?)
    .map_err(|e| format!("bad partial_crossing_fractions_bps: {e}"))?;
    if doc_part != partial_crossing_fractions_bps {
        return Err(format!(
            "partial_crossing_fractions_bps mismatch: doc={doc_part:?} default={partial_crossing_fractions_bps:?}"
        ));
    }

    // 10. debt_repayment_grid_bps
    let doc_debt: Vec<u16> = serde_json::from_str(&first_backticked(
        get("debt_repayment_grid_bps")?,
        "debt_repayment_grid_bps",
    )?)
    .map_err(|e| format!("bad debt_repayment_grid_bps: {e}"))?;
    if doc_debt != debt_repayment_grid_bps {
        return Err(format!(
            "debt_repayment_grid_bps mismatch: doc={doc_debt:?} default={debt_repayment_grid_bps:?}"
        ));
    }

    // 11. quote_offsets_bps
    let doc_quotes: Vec<i16> = serde_json::from_str(&first_backticked(
        get("quote_offsets_bps")?,
        "quote_offsets_bps",
    )?)
    .map_err(|e| format!("bad quote_offsets_bps: {e}"))?;
    if doc_quotes != quote_offsets_bps {
        return Err(format!(
            "quote_offsets_bps mismatch: doc={doc_quotes:?} default={quote_offsets_bps:?}"
        ));
    }

    // 12. quantity_scales_bps
    let doc_scales: Vec<u16> = serde_json::from_str(&first_backticked(
        get("quantity_scales_bps")?,
        "quantity_scales_bps",
    )?)
    .map_err(|e| format!("bad quantity_scales_bps: {e}"))?;
    if doc_scales != quantity_scales_bps {
        return Err(format!(
            "quantity_scales_bps mismatch: doc={doc_scales:?} default={quantity_scales_bps:?}"
        ));
    }

    // 13. composite_greedy_rules
    let doc_greedy = unquote_doc_str(
        &first_backticked(get("composite_greedy_rules")?, "composite_greedy_rules")?,
        "composite_greedy_rules",
    )?;
    if doc_greedy != composite_greedy_rules {
        return Err(format!(
            "composite_greedy_rules mismatch: doc={doc_greedy} default={composite_greedy_rules}"
        ));
    }

    // 14. synth_slot_fill_order
    let doc_slots: Vec<String> = serde_json::from_str(&first_backticked(
        get("synth_slot_fill_order")?,
        "synth_slot_fill_order",
    )?)
    .map_err(|e| format!("bad synth_slot_fill_order: {e}"))?;
    if doc_slots != synth_slot_fill_order {
        return Err(format!(
            "synth_slot_fill_order mismatch: doc={doc_slots:?} default={synth_slot_fill_order:?}"
        ));
    }

    // 15. upkeep_cover_turns
    let doc_upkeep: u8 = first_backticked(get("upkeep_cover_turns")?, "upkeep_cover_turns")?
        .parse()
        .map_err(|e| format!("bad upkeep_cover_turns: {e}"))?;
    if doc_upkeep != upkeep_cover_turns {
        return Err(format!(
            "upkeep_cover_turns mismatch: doc={doc_upkeep} default={upkeep_cover_turns}"
        ));
    }

    // 16. parity_spread_threshold_bps
    let doc_spread: u16 = first_backticked(
        get("parity_spread_threshold_bps")?,
        "parity_spread_threshold_bps",
    )?
    .parse()
    .map_err(|e| format!("bad parity_spread_threshold_bps: {e}"))?;
    if doc_spread != parity_spread_threshold_bps {
        return Err(format!(
            "parity_spread_threshold_bps mismatch: doc={doc_spread} default={parity_spread_threshold_bps}"
        ));
    }

    // 17–18. talk_extractor_version & talk_extractor_rules_digest
    let talk_v_cell = get("talk_extractor_version")?;
    let talk_d_cell = get("talk_extractor_rules_digest")?;
    if talk_v_cell != talk_d_cell {
        return Err("talk_extractor_version and talk_extractor_rules_digest cells disagree".into());
    }
    let talk_tokens = extract_backticked_tokens(talk_v_cell);
    if talk_tokens.len() < 2 {
        return Err(format!(
            "expected version and digest backticked tokens in `{talk_v_cell}`"
        ));
    }
    let doc_talk_ver = unquote_doc_str(talk_tokens[0], "talk_extractor_version")?;
    if doc_talk_ver != talk_extractor_version {
        return Err(format!(
            "talk_extractor_version mismatch: doc={doc_talk_ver} default={talk_extractor_version}"
        ));
    }
    let doc_talk_digest = unquote_doc_str(talk_tokens[1], "talk_extractor_rules_digest")?;
    if !is_valid_sha256_hex(&doc_talk_digest) || doc_talk_digest != talk_extractor_rules_digest {
        return Err(format!(
            "talk_extractor_rules_digest mismatch: doc={doc_talk_digest} default={talk_extractor_rules_digest}"
        ));
    }

    // 19. mandate_stratification_mode
    let doc_mode = unquote_doc_str(
        &first_backticked(
            get("mandate_stratification_mode")?,
            "mandate_stratification_mode",
        )?,
        "mandate_stratification_mode",
    )?;
    if doc_mode != mandate_stratification_mode {
        return Err(format!(
            "mandate_stratification_mode mismatch: doc={doc_mode} default={mandate_stratification_mode}"
        ));
    }

    // 20. prescreen_min_arm_indep_span
    let doc_span: i64 = first_backticked(
        get("prescreen_min_arm_indep_span")?,
        "prescreen_min_arm_indep_span",
    )?
    .parse()
    .map_err(|e| format!("bad prescreen_min_arm_indep_span: {e}"))?;
    if doc_span != prescreen_min_arm_indep_span {
        return Err(format!(
            "prescreen_min_arm_indep_span mismatch: doc={doc_span} default={prescreen_min_arm_indep_span}"
        ));
    }

    // 21. prescreen_exclude_flat_sets
    let doc_flat: bool = first_backticked(
        get("prescreen_exclude_flat_sets")?,
        "prescreen_exclude_flat_sets",
    )?
    .parse()
    .map_err(|e| format!("bad prescreen_exclude_flat_sets: {e}"))?;
    if doc_flat != prescreen_exclude_flat_sets {
        return Err(format!(
            "prescreen_exclude_flat_sets mismatch: doc={doc_flat} default={prescreen_exclude_flat_sets}"
        ));
    }

    // 22. prescreen_min_arm_indep_candidates
    let doc_cands: usize = first_backticked(
        get("prescreen_min_arm_indep_candidates")?,
        "prescreen_min_arm_indep_candidates",
    )?
    .parse()
    .map_err(|e| format!("bad prescreen_min_arm_indep_candidates: {e}"))?;
    if doc_cands != prescreen_min_arm_indep_candidates {
        return Err(format!(
            "prescreen_min_arm_indep_candidates mismatch: doc={doc_cands} default={prescreen_min_arm_indep_candidates}"
        ));
    }

    // 23. strata_schedule
    let doc_strata = unquote_doc_str(
        &first_backticked(get("strata_schedule")?, "strata_schedule")?,
        "strata_schedule",
    )?;
    if doc_strata != strata_schedule {
        return Err(format!(
            "strata_schedule mismatch: doc={doc_strata} default={strata_schedule}"
        ));
    }

    Ok(())
}

fn parse_sec52_fixed_constants(sec52: &str) -> Result<BTreeMap<String, String>, String> {
    let para_start = sec52
        .find("*Proposed fixed code constants")
        .ok_or("missing fixed constants paragraph in §5.2")?;
    let para_end = sec52[para_start..]
        .find("**Build requirement:**")
        .map(|off| para_start + off)
        .ok_or("missing Build requirement end marker in §5.2")?;
    let para = &sec52[para_start..para_end];

    let mut map = BTreeMap::new();
    for tok in extract_backticked_tokens(para) {
        if let Some((lhs, rhs)) = tok.split_once('=') {
            let name = lhs.trim().rsplit("::").next().unwrap_or("").trim();
            let val = rhs.trim();
            if !name.is_empty()
                && name.bytes().any(|b| b.is_ascii_uppercase())
                && name
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                && map.insert(name.to_owned(), val.to_owned()).is_some()
            {
                return Err(format!("duplicate constant `{name}` in §5.2"));
            }
        }
    }
    Ok(map)
}

fn verify_scorer_constants_documented(sources: &[(&str, &str)], sec52: &str) -> Result<(), String> {
    const ALLOWED_SCHEMA_AND_TABLE_CONSTS: [&str; 3] = [
        "BLIND_SCORING_SCHEMA_VERSION",
        "EVAL_TABLE_SEAT_COUNT",
        "SYNTH_SLOT_BUCKET_NAMES",
    ];
    const PREFIXES: [&str; 4] = [
        "pub const ",
        "pub(crate) const ",
        "pub static ",
        "pub(crate) static ",
    ];
    let documented = parse_sec52_fixed_constants(sec52)?;
    let mut found_count = 0_usize;
    for &(file_label, src) in sources {
        for raw_line in src.lines() {
            let trimmed = raw_line.trim_start();
            for prefix in PREFIXES {
                if let Some(after) = trimmed.strip_prefix(prefix)
                    && let Some((name_raw, _)) = after.split_once(':')
                {
                    let name = name_raw.trim();
                    found_count += 1;
                    if !documented.contains_key(name)
                        && !ALLOWED_SCHEMA_AND_TABLE_CONSTS.contains(&name)
                    {
                        return Err(format!(
                            "constant `{name}` in {file_label} is missing from docs/decision-scorer.md §5.2"
                        ));
                    }
                }
            }
        }
    }
    if found_count == 0 {
        return Err(
            "failed to enumerate any constant/static declarations in scorer sources".to_owned(),
        );
    }
    Ok(())
}

fn verify_sec52_constants_match(sec52: &str) -> Result<(), String> {
    let constants = parse_sec52_fixed_constants(sec52)?;
    const EXPECTED_CONSTANT_COUNT: usize = 22;
    if constants.len() != EXPECTED_CONSTANT_COUNT {
        return Err(format!(
            "§5.2 constants count mismatch: got {}, expected {EXPECTED_CONSTANT_COUNT}",
            constants.len()
        ));
    }
    let check_num = |name: &str, actual: &str| -> Result<(), String> {
        let raw = constants
            .get(name)
            .ok_or_else(|| format!("§5.2 constants missing `{name}`"))?;
        let normalized_doc = raw.replace('_', "");
        let normalized_actual = actual.replace('_', "");
        if normalized_doc != normalized_actual {
            return Err(format!(
                "constant `{name}` mismatch: doc=`{raw}` code=`{actual}`"
            ));
        }
        Ok(())
    };
    let check_exact = |name: &str, expected: &str| -> Result<(), String> {
        let raw = constants
            .get(name)
            .ok_or_else(|| format!("§5.2 constants missing `{name}`"))?;
        if raw != expected {
            return Err(format!(
                "constant `{name}` mismatch: doc=`{raw}` expected=`{expected}`"
            ));
        }
        Ok(())
    };

    check_num(
        "SYNTH_MAX_CONVERTER_BATCHES",
        &SYNTH_MAX_CONVERTER_BATCHES.to_string(),
    )?;
    check_num(
        "SYNTH_MAX_PROJECT_BATCHES",
        &SYNTH_MAX_PROJECT_BATCHES.to_string(),
    )?;
    check_exact(
        "SYNTH_PROJECT_RANK_BY_EQUITY",
        &SYNTH_PROJECT_RANK_BY_EQUITY.to_string(),
    )?;
    check_exact(
        "SYNTH_CONVERTER_INPUT_DEFICIT_COVER",
        &SYNTH_CONVERTER_INPUT_DEFICIT_COVER.to_string(),
    )?;
    check_exact(
        "SYNTH_CLAIM_MATURING_EXACT",
        &SYNTH_CLAIM_MATURING_EXACT.to_string(),
    )?;
    check_exact(
        "SYNTH_COUNCIL_SELF_NOMINATION",
        &SYNTH_COUNCIL_SELF_NOMINATION.to_string(),
    )?;
    check_exact(
        "DROP_TIE_BREAK_BY_CANDIDATE_ID",
        &DROP_TIE_BREAK_BY_CANDIDATE_ID.to_string(),
    )?;
    check_exact(
        "COMPOSITE_GREEDY_RULES_DEFAULT",
        &format!("\"{COMPOSITE_GREEDY_RULES_DEFAULT}\""),
    )?;
    check_num(
        "SYNTH_CHARTER_BID_CRED",
        &SYNTH_CHARTER_BID_CRED.to_string(),
    )?;
    check_num(
        "SYNTH_QUOTE_CRED_SHARE_BPS",
        &SYNTH_QUOTE_CRED_SHARE_BPS.to_string(),
    )?;
    check_num("UNIT_SCALE", &UNIT_SCALE.to_string())?;
    check_num("BPS_SCALE", &league_core::BPS_SCALE.to_string())?;
    assert_eq!(SYNTH_QUOTE_MAX_UNITS, UNIT_SCALE);
    check_exact("SYNTH_QUOTE_MAX_UNITS", "UNIT_SCALE")?;
    check_num("SYNTH_QUOTE_MIN_UNITS", &SYNTH_QUOTE_MIN_UNITS.to_string())?;
    check_num(
        "SYNTH_DROP_UPKEEP_SELL_RISK_MULTIPLIER",
        &SYNTH_DROP_UPKEEP_SELL_RISK_MULTIPLIER.to_string(),
    )?;
    check_num(
        "SYNTH_DROP_DEFAULT_ACTION_RISK_CRED",
        &SYNTH_DROP_DEFAULT_ACTION_RISK_CRED.to_string(),
    )?;
    assert_eq!(SYNTH_DROP_PROJECT_RISK_CRED, i64::MAX / 2);
    check_exact("SYNTH_DROP_PROJECT_RISK_CRED", "i64::MAX / 2")?;
    check_num(
        "SYNTH_SCALE_SEED_PER_AXIS",
        &SYNTH_SCALE_SEED_PER_AXIS.to_string(),
    )?;
    check_num(
        "SYNTH_SCALE_SEED_POOL_MAX",
        &SYNTH_SCALE_SEED_POOL_MAX.to_string(),
    )?;
    check_num("HONOUR_PPM_SCALE", &HONOUR_PPM_SCALE.to_string())?;
    check_exact(
        "TALK_EXTRACTOR_VERSION_V2",
        &format!("\"{TALK_EXTRACTOR_VERSION_V2}\""),
    )?;
    check_exact(
        "TALK_EXTRACTOR_RULES_DIGEST_V2",
        &format!("\"{TALK_EXTRACTOR_RULES_DIGEST_V2}\""),
    )?;
    Ok(())
}

#[test]
fn scorer_config_default_and_constants_match_design_doc_and_club_bot() -> TestResult {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let doc_path = manifest_dir.join("../../docs/decision-scorer.md");
    let doc = fs::read_to_string(&doc_path)?;
    let sec52_start = doc
        .find("### 5.2 Complete Pre-Registered Free Parameter Table")
        .ok_or("missing §5.2 heading in docs/decision-scorer.md")?;
    let sec6_start = doc[sec52_start..]
        .find("## 6. Horizon Valuation")
        .map(|off| sec52_start + off)
        .ok_or("missing §6 heading after §5.2")?;
    let sec52 = &doc[sec52_start..sec6_start];
    let types_src = fs::read_to_string(manifest_dir.join("src/types.rs"))?;
    let candidates_src = fs::read_to_string(manifest_dir.join("src/candidates.rs"))?;
    let postures_src = fs::read_to_string(manifest_dir.join("src/postures.rs"))?;
    let scenarios_src = fs::read_to_string(manifest_dir.join("src/scenarios.rs"))?;

    let cfg = ScorerConfig::default();
    assert!(cfg.validate().is_ok());
    verify_sec52_table_matches_defaults(sec52)?;
    verify_sec52_constants_match(sec52)?;
    verify_scorer_constants_documented(
        &[
            ("types.rs", &types_src),
            ("candidates.rs", &candidates_src),
            ("postures.rs", &postures_src),
            ("scenarios.rs", &scenarios_src),
        ],
        sec52,
    )?;

    // Negative controls proving doc drift, removed rows, or unlisted constants fail with exact error reasons:
    let mutated_m = sec52.replacen("| `scenarios_m` (`M`) | Number of CRN scenarios per state (sized from pilot measured SE/diversity) | `16` |", "| `scenarios_m` (`M`) | Number of CRN scenarios per state (sized from pilot measured SE/diversity) | `32` |", 1);
    assert_ne!(mutated_m, sec52);
    let err_m = verify_sec52_table_matches_defaults(&mutated_m)
        .err()
        .ok_or("expected mutated scenarios_m row to fail")?;
    assert!(
        err_m.contains("scenarios_m mismatch"),
        "unexpected: {err_m}"
    );
    assert!(err_m.contains("doc=32 default=16"), "unexpected: {err_m}");
    eprintln!("negative_control[value_changed:scenarios_m]: {err_m}");

    let mutated_bool = sec52.replacen(
        "| `include_pass_b_cleared_club_bot` | Include `ClubBot` decision with `pass_b_actions` cleared (counted inside `k_synth_max`) | `true` |",
        "| `include_pass_b_cleared_club_bot` | Include `ClubBot` decision with `pass_b_actions` cleared (counted inside `k_synth_max`) | `false` |",
        1,
    );
    assert_ne!(mutated_bool, sec52);
    let err_bool = verify_sec52_table_matches_defaults(&mutated_bool)
        .err()
        .ok_or("expected mutated bool row to fail")?;
    assert!(
        err_bool.contains("include_pass_b_cleared_club_bot mismatch"),
        "unexpected: {err_bool}"
    );
    assert!(
        err_bool.contains("doc=false default=true"),
        "unexpected: {err_bool}"
    );
    eprintln!("negative_control[value_changed:include_pass_b_cleared_club_bot]: {err_bool}");

    let removed_row: String = sec52
        .lines()
        .filter(|l| !l.contains("`upkeep_cover_turns`"))
        .collect::<Vec<_>>()
        .join("\n");
    let err_removed = verify_sec52_table_matches_defaults(&removed_row)
        .err()
        .ok_or("expected removed upkeep_cover_turns row to fail")?;
    assert!(
        err_removed.contains("mapped 20 fields, expected 21"),
        "unexpected: {err_removed}"
    );
    eprintln!("negative_control[row_removed:upkeep_cover_turns]: {err_removed}");

    let mutated_const = sec52.replacen(
        "`SYNTH_CHARTER_BID_CRED = 150`",
        "`SYNTH_CHARTER_BID_CRED = 151`",
        1,
    );
    let err_const_val = verify_sec52_constants_match(&mutated_const)
        .err()
        .ok_or("expected mutated constant value to fail")?;
    assert!(
        err_const_val.contains("SYNTH_CHARTER_BID_CRED")
            && err_const_val.contains("doc=`151` code=`150`"),
        "unexpected: {err_const_val}"
    );
    eprintln!("negative_control[const_changed:SYNTH_CHARTER_BID_CRED]: {err_const_val}");

    let mutated_unit_scale = sec52.replacen(
        "`league_seat_types::UNIT_SCALE = 1_000`",
        "`league_seat_types::UNIT_SCALE = 999`",
        1,
    );
    let err_unit_scale = verify_sec52_constants_match(&mutated_unit_scale)
        .err()
        .ok_or("expected mutated UNIT_SCALE constant to fail")?;
    assert!(
        err_unit_scale.contains("UNIT_SCALE") && err_unit_scale.contains("doc=`999` code=`1000`"),
        "unexpected: {err_unit_scale}"
    );
    eprintln!("negative_control[const_changed:UNIT_SCALE]: {err_unit_scale}");

    let removed_const = sec52.replacen("`HONOUR_PPM_SCALE = 1_000_000`", "`REMOVED_NONE`", 1);
    let err_const_rm = verify_sec52_constants_match(&removed_const)
        .err()
        .ok_or("expected removed constant to fail")?;
    assert!(
        err_const_rm.contains("got 21, expected 22"),
        "unexpected: {err_const_rm}"
    );
    eprintln!("negative_control[const_removed:HONOUR_PPM_SCALE]: {err_const_rm}");

    let mutated_talk_digest = sec52.replacen(TALK_EXTRACTOR_RULES_DIGEST_V2, &"0".repeat(64), 2);
    let err_talk_table = verify_sec52_table_matches_defaults(&mutated_talk_digest)
        .err()
        .ok_or("expected mutated talk digest row to fail")?;
    assert!(
        err_talk_table.contains("talk_extractor_rules_digest mismatch"),
        "unexpected: {err_talk_table}"
    );
    let err_talk_const = verify_sec52_constants_match(&mutated_talk_digest)
        .err()
        .ok_or("expected mutated talk digest constant to fail")?;
    assert!(
        err_talk_const.contains("TALK_EXTRACTOR_RULES_DIGEST_V2"),
        "unexpected: {err_talk_const}"
    );

    let unlisted_types = format!("{types_src}\npub const UNLISTED_MAGIC: u64 = 42;\n");
    let err_unlisted = verify_scorer_constants_documented(&[("types.rs", &unlisted_types)], sec52)
        .err()
        .ok_or("expected unlisted types.rs pub const to fail")?;
    assert!(
        err_unlisted.contains("UNLISTED_MAGIC") && err_unlisted.contains("missing from"),
        "unexpected: {err_unlisted}"
    );
    eprintln!("negative_control[unlisted_pub_const:UNLISTED_MAGIC]: {err_unlisted}");

    for (label, decl, expected_name) in [
        (
            "candidates.rs",
            "pub(crate) const UNLISTED_CRATE_CONST: u64 = 7;\n",
            "UNLISTED_CRATE_CONST",
        ),
        (
            "postures.rs",
            "pub static UNLISTED_POSTURE_STATIC: u64 = 9;\n",
            "UNLISTED_POSTURE_STATIC",
        ),
        (
            "scenarios.rs",
            "pub(crate) static UNLISTED_SCENARIO_STATIC: u64 = 11;\n",
            "UNLISTED_SCENARIO_STATIC",
        ),
    ] {
        let err_mod = verify_scorer_constants_documented(&[(label, decl)], sec52)
            .err()
            .ok_or("expected unlisted constant/static to fail")?;
        assert!(
            err_mod.contains(expected_name) && err_mod.contains(label),
            "unexpected for {label}: {err_mod}"
        );
    }

    // Pin SYNTH_CHARTER_BID_CRED to ClubBot's charter auction bid on Turn 6.
    let mut host = EvalHost::from_standard_setup(TableId(8261), 4, 1, 0)?;
    while host.turn() < 6 && host.phase() == TurnPhase::TalkAndMarket {
        let mut subs = BTreeMap::new();
        for s in 0..4_u8 {
            let obs = host.observe_seat(Seat(s))?;
            let mandate = *obs.own_mandate().ok_or("missing mandate")?;
            let dec =
                ClubBot::new(100_u64.wrapping_add(u64::from(s))).decide_full_turn(&obs, &mandate);
            subs.insert(
                s,
                EvalActSubmission {
                    market_actions: dec.pass_a_actions,
                    pass_b_actions: dec.pass_b_actions,
                    bundle: dec.bundle,
                },
            );
        }
        host.apply_act_submissions(&subs)?;
        host.sweep_turn()?;
    }
    assert_eq!(host.turn(), 6);
    let mut obs6 = host.observe_seat(Seat(0))?;
    let mandate6 = *obs6.own_mandate().ok_or("missing turn-6 mandate")?;
    obs6.seats[0].cred = 1_000_000;
    let bot_dec6 = ClubBot::new(1).decide_full_turn(&obs6, &mandate6);
    let bot_charter_bid = bot_dec6
        .bundle
        .charter_bids
        .first()
        .ok_or("expected ClubBot charter_bid on Turn 6")?;
    assert_eq!(bot_charter_bid.bid_cred, SYNTH_CHARTER_BID_CRED);
    let cands6 = generate_arm_independent_candidates(&obs6, &mandate6, &cfg, 1)?;
    assert!(
        cands6.iter().any(|c| {
            c.submission
                .bundle
                .charter_bids
                .iter()
                .any(|b| b.bid_cred == SYNTH_CHARTER_BID_CRED)
        }),
        "expected synthetic charter bid matching SYNTH_CHARTER_BID_CRED"
    );
    Ok(())
}

#[test]
fn preview_error_fails_closed_across_pruning_and_commitments() -> TestResult {
    let host = EvalHost::from_standard_setup(TableId(8262), 4, 1, 0)?;
    let mut bad_obs = host.observe_seat(Seat(1))?;
    bad_obs.seat = 99;
    let dummy_action = MarketAction::TownImport {
        good: AssetId::Raw(0),
        seat: 99,
        amount: 10,
    };
    let prune_err = prune_refused_market_actions(
        &bad_obs,
        vec![dummy_action.clone()],
        &SealedOrderBundle::default(),
    )
    .err()
    .ok_or("expected ENGINE_ERROR from prune_refused_market_actions on invalid seat")?;
    assert_eq!(prune_err.code(), "ENGINE_ERROR");

    let comm = [ExtractedTalkCommitment {
        counterparty_seat: 0,
        offered_action: dummy_action,
        proposer_seat: 99,
    }];
    let pair_sha = sha256_hex(b"fail-closed");
    let apply_err = apply_talk_commitments(
        &bad_obs,
        EvalActSubmission::default(),
        &comm,
        1,
        &pair_sha,
        0,
        1.0,
    )
    .err()
    .ok_or("expected ENGINE_ERROR from apply_talk_commitments on invalid seat")?;
    assert_eq!(apply_err.code(), "ENGINE_ERROR");

    let obs_valid = host.observe_seat(Seat(1))?;
    let bad_p_err = apply_talk_commitments(
        &obs_valid,
        EvalActSubmission::default(),
        &comm,
        1,
        &pair_sha,
        0,
        f64::NAN,
    )
    .err()
    .ok_or("expected INVALID_SCORER_CONFIG on NaN p_honour")?;
    assert_eq!(bad_p_err.code(), "INVALID_SCORER_CONFIG");
    Ok(())
}

#[test]
fn rollout_replay_parity_covers_active_conservative_and_honoured_commitments() -> TestResult {
    let tmp = UniqueTmpDir::new("replay_parity_s2")?;
    let mut host = EvalHost::from_standard_setup(TableId(8263), 4, 1, 0)?
        .with_engine_commit(TEST_ENGINE_COMMIT)?;

    // Turn 1: play unmodified ClubBot.
    let mut t1_subs = BTreeMap::new();
    for s in 0..4_u8 {
        let obs = host.observe_seat(Seat(s))?;
        let mandate = *obs.own_mandate().ok_or("missing Turn-1 mandate")?;
        let dec = ClubBot::new(50_u64.wrapping_add(u64::from(s))).decide_full_turn(&obs, &mandate);
        t1_subs.insert(
            s,
            EvalActSubmission {
                market_actions: dec.pass_a_actions,
                pass_b_actions: dec.pass_b_actions,
                bundle: dec.bundle,
            },
        );
    }
    host.apply_act_submissions(&t1_subs)?;
    host.sweep_turn()?;

    // Turn 2 checkpoint: verify Conservative actually diverges from Standard on Turn 2 under default config.
    let ckpt_t2 = tmp.path().join("ckpt_t2");
    fs::create_dir_all(&ckpt_t2)?;
    let (save_t2, seal_t2) = host.write_pre_act_checkpoint(&ckpt_t2)?;
    let save_t2_bytes = fs::read(&save_t2)?;
    let seal_t2_bytes = fs::read(&seal_t2)?;
    let save_t2_sha = sha256_hex(&save_t2_bytes);
    let seal_t2_sha = sha256_hex(&seal_t2_bytes);
    let pair_t2_sha = compute_pre_act_pair_sha256_hex(&save_t2_bytes, &seal_t2_bytes);

    let focal_t2 = host.observe_seat(Seat(0))?;
    let focal_mandate_t2 = *focal_t2
        .own_mandate()
        .ok_or("missing focal Turn-2 mandate")?;
    let scenarios_t2 = draw_scenarios(
        &focal_t2.spec,
        focal_mandate_t2,
        0,
        99,
        &pair_t2_sha,
        host.turn(),
        2,
    )?;
    let cfg = ScorerConfig::default();

    let mut std_t2_subs = BTreeMap::new();
    let mut cons_t2_subs = BTreeMap::new();
    for s in 1..4_u8 {
        let obs_s = host.observe_seat(Seat(s))?;
        let m_s = *scenarios_t2[0]
            .peer_mandates
            .get(&s)
            .ok_or("missing Turn-2 peer mandate")?;
        let seed_s = scenarios_t2[0].bot_base_seed.wrapping_add(u64::from(s));
        std_t2_subs.insert(
            s,
            decide_peer_turn(PostureKind::Standard, seed_s, &obs_s, &m_s, &cfg)?,
        );
        cons_t2_subs.insert(
            s,
            decide_peer_turn(PostureKind::Conservative, seed_s, &obs_s, &m_s, &cfg)?,
        );
    }
    assert!(
        (1..4_u8).any(|s| cons_t2_subs[&s] != std_t2_subs[&s]),
        "expected Conservative Turn-day submission to differ from Standard for at least one peer"
    );

    let (cons_outcome, cons_log) = run_single_rollout_with_replay_log(
        &save_t2,
        &seal_t2,
        (&save_t2_sha, &seal_t2_sha, &pair_t2_sha),
        0,
        &EvalActSubmission::default(),
        &cons_t2_subs,
        focal_mandate_t2,
        &scenarios_t2[0],
        PostureKind::Conservative,
        &cfg,
    )?;
    let cons_replayed = replay_eval_season(&cons_log)?;
    let cons_row0 = cons_replayed
        .scoreboard
        .standings
        .iter()
        .find(|e| e.seat == 0)
        .ok_or("missing Conservative replay seat 0")?;
    assert_eq!(cons_outcome.total_equity, cons_row0.total_equity);
    assert_eq!(cons_outcome.project_equity, cons_row0.project_equity);
    assert_eq!(cons_outcome.net_asset_equity, cons_row0.net_asset_equity);
    assert_eq!(cons_outcome.mandate_bonus, cons_row0.mandate_bonus);

    // Turn 2 -> Turn 3: seat 2 posts an ask-free bid on `wide_g` (`best_ask == None`) so Active
    // posture diverges from Standard on Turn 3 across peers holding spare `wide_g`.
    let probe1 = host.observe_seat(Seat(1))?;
    let wide_g = (0..4_u8)
        .map(AssetId::Raw)
        .find(|&g| matches!(g, AssetId::Raw(idx) if probe1.seats[1].raw[usize::from(idx)] >= 200))
        .ok_or("seat 1 lacks >= 200 raw units for Turn-3 Active crossing")?;
    let mut t2_subs = BTreeMap::new();
    for s in 0..4_u8 {
        let obs_s = host.observe_seat(Seat(s))?;
        let p_wide = obs_s.parities.asset_parity(wide_g);
        let mut actions = Vec::new();
        if s == 2 {
            actions.push(MarketAction::PostOffer {
                seat: 2,
                order: OfferProposal {
                    side: OfferSide::Buy,
                    good: wide_g,
                    amount: 200,
                    price_per_unit_cred: (p_wide * 80) / 100,
                    domain_id: None,
                    is_hybrid: false,
                },
            });
        }
        t2_subs.insert(
            s,
            EvalActSubmission {
                market_actions: actions,
                pass_b_actions: Vec::new(),
                bundle: SealedOrderBundle::default(),
            },
        );
    }
    host.apply_act_submissions(&t2_subs)?;
    host.sweep_turn()?;

    let ckpt_t3 = tmp.path().join("ckpt_t3");
    fs::create_dir_all(&ckpt_t3)?;
    let (save_t3, seal_t3) = host.write_pre_act_checkpoint(&ckpt_t3)?;
    let save_t3_bytes = fs::read(&save_t3)?;
    let seal_t3_bytes = fs::read(&seal_t3)?;
    let save_t3_sha = sha256_hex(&save_t3_bytes);
    let seal_t3_sha = sha256_hex(&seal_t3_bytes);
    let pair_t3_sha = compute_pre_act_pair_sha256_hex(&save_t3_bytes, &seal_t3_bytes);

    let day_t3 = host.turn();
    let focal_t3 = host.observe_seat(Seat(0))?;
    let focal_mandate_t3 = *focal_t3
        .own_mandate()
        .ok_or("missing focal Turn-3 mandate")?;
    let scenarios_t3 = draw_scenarios(
        &focal_t3.spec,
        focal_mandate_t3,
        0,
        99,
        &pair_t3_sha,
        day_t3,
        2,
    )?;

    let mut std_t3_subs = BTreeMap::new();
    let mut act_t3_subs = BTreeMap::new();
    for s in 1..4_u8 {
        let obs_s = host.observe_seat(Seat(s))?;
        let m_s = *scenarios_t3[0]
            .peer_mandates
            .get(&s)
            .ok_or("missing Turn-3 peer mandate")?;
        let seed_s = scenarios_t3[0].bot_base_seed.wrapping_add(u64::from(s));
        std_t3_subs.insert(
            s,
            decide_peer_turn(PostureKind::Standard, seed_s, &obs_s, &m_s, &cfg)?,
        );
        act_t3_subs.insert(
            s,
            decide_peer_turn(PostureKind::Active, seed_s, &obs_s, &m_s, &cfg)?,
        );
    }
    assert!(
        (1..4_u8).any(|s| act_t3_subs[&s] != std_t3_subs[&s]),
        "expected Active Turn-day submission to differ from Standard for at least one peer"
    );

    let (act_outcome, act_log) = run_single_rollout_with_replay_log(
        &save_t3,
        &seal_t3,
        (&save_t3_sha, &seal_t3_sha, &pair_t3_sha),
        0,
        &EvalActSubmission::default(),
        &act_t3_subs,
        focal_mandate_t3,
        &scenarios_t3[0],
        PostureKind::Active,
        &cfg,
    )?;
    let act_replayed = replay_eval_season(&act_log)?;
    let act_row0 = act_replayed
        .scoreboard
        .standings
        .iter()
        .find(|e| e.seat == 0)
        .ok_or("missing Active replay seat 0")?;
    assert_eq!(act_outcome.total_equity, act_row0.total_equity);
    assert_eq!(act_outcome.project_equity, act_row0.project_equity);
    assert_eq!(act_outcome.net_asset_equity, act_row0.net_asset_equity);
    assert_eq!(act_outcome.mandate_bonus, act_row0.mandate_bonus);

    // Verify independent replay parity and exact Turn-day log entry when a talk commitment is honoured (p_honour = 1.0).
    let honoured_action = MarketAction::TownImport {
        good: AssetId::Raw(0),
        seat: 1,
        amount: 25,
    };
    let commitments = canonicalize_talk_commitments(&[ExtractedTalkCommitment {
        counterparty_seat: 2,
        offered_action: honoured_action.clone(),
        proposer_seat: 1,
    }])?;
    let mut honoured_peer_subs = BTreeMap::new();
    for s in 1..4_u8 {
        let obs_s = host.observe_seat(Seat(s))?;
        let m_s = *scenarios_t3[1]
            .peer_mandates
            .get(&s)
            .ok_or("missing Turn-3 scenario-1 peer mandate")?;
        let base_s = decide_peer_turn(
            PostureKind::Standard,
            scenarios_t3[1].bot_base_seed.wrapping_add(u64::from(s)),
            &obs_s,
            &m_s,
            &cfg,
        )?;
        let applied_s =
            apply_talk_commitments(&obs_s, base_s, &commitments, 99, &pair_t3_sha, 1, 1.0)?;
        honoured_peer_subs.insert(s, applied_s);
    }
    assert!(
        honoured_peer_subs
            .get(&1)
            .is_some_and(|s| s.market_actions.contains(&honoured_action)),
        "expected honoured TownImport commitment in Seat 1 Turn-day submission"
    );

    let (honoured_outcome, honoured_log) = run_single_rollout_with_replay_log(
        &save_t3,
        &seal_t3,
        (&save_t3_sha, &seal_t3_sha, &pair_t3_sha),
        0,
        &EvalActSubmission::default(),
        &honoured_peer_subs,
        focal_mandate_t3,
        &scenarios_t3[1],
        PostureKind::Standard,
        &cfg,
    )?;
    let day_turn = honoured_log
        .turns
        .iter()
        .find(|t| t.turn == day_t3)
        .ok_or("missing Turn-day entry in honoured SeasonReplayLog")?;
    assert!(
        day_turn.market_actions.contains(&honoured_action),
        "expected specific honoured TownImport action recorded in Turn-{day_t3} replay log"
    );
    let replayed_honoured = replay_eval_season(&honoured_log)?;
    let replay_honoured_row0 = replayed_honoured
        .scoreboard
        .standings
        .iter()
        .find(|e| e.seat == 0)
        .ok_or("missing honoured replay seat 0")?;
    assert_eq!(
        honoured_outcome.total_equity,
        replay_honoured_row0.total_equity
    );
    assert_eq!(
        honoured_outcome.project_equity,
        replay_honoured_row0.project_equity
    );
    assert_eq!(
        honoured_outcome.net_asset_equity,
        replay_honoured_row0.net_asset_equity
    );
    assert_eq!(
        honoured_outcome.mandate_bonus,
        replay_honoured_row0.mandate_bonus
    );

    // Missing peer mandate in scenario fails closed cleanly inside run_single_rollout_with_replay_log.
    let mut bad_scenario = scenarios_t3[0].clone();
    bad_scenario.peer_mandates.remove(&1);
    let missing_mandate_err = run_single_rollout_with_replay_log(
        &save_t3,
        &seal_t3,
        (&save_t3_sha, &seal_t3_sha, &pair_t3_sha),
        0,
        &EvalActSubmission::default(),
        &honoured_peer_subs,
        focal_mandate_t3,
        &bad_scenario,
        PostureKind::Standard,
        &cfg,
    )
    .err()
    .ok_or("expected ENGINE_ERROR when peer mandate is missing")?;
    assert_eq!(missing_mandate_err.code(), "ENGINE_ERROR");
    Ok(())
}

#[test]
fn composite_drops_prices_upkeep_sell_in_milli_cred_parity() -> TestResult {
    let host = EvalHost::from_standard_setup(TableId(8264), 4, 1, 0)?;
    let mut obs = host.observe_seat(Seat(0))?;
    let mandate = *obs.own_mandate().ok_or("missing mandate")?;
    obs.spec.seats[0].converters.truncate(1);
    obs.spec.seats[0].converters[0].inputs.truncate(1);
    let seat_cfg = obs.spec.seats[0].clone();
    let upkeep_idx = usize::from(seat_cfg.upkeep_good);
    let (raw_idx, per_run) = seat_cfg
        .converters
        .first()
        .and_then(|r| r.inputs.first().copied())
        .ok_or("expected converter recipe raw input on seat 0")?;

    // Set upkeep_good parity = 200 (!= UNIT_SCALE = 1_000) and converter raw parity high enough that:
    //   new_upkeep_risk = mul_units_parity(1_000, 200) * 2 = 400
    //   < import_cost (from TownImport of converter input)
    //   < old_upkeep_risk = 1_000 * 2 = 2_000.
    // Because build_single_dropped_variant drops the HIGHEST-risk leg first:
    // - New code: drops TownImport, leaving PostOffer(Sell, upkeep_good) alone.
    // - Old code: dropped PostOffer(Sell, upkeep_good), leaving TownImport alone.
    obs.turn = 1;
    obs.offers.clear();
    obs.parities.refined_per_unit[upkeep_idx] = 200;
    obs.parities.raw_per_unit.fill(250);
    obs.seats[0].cred = 100_000;
    obs.seats[0].cred_debt = 0;
    obs.seats[0].raw.fill(0);
    obs.seats[0].refined.fill(0);

    let cfg = ScorerConfig {
        include_pass_b_cleared_club_bot: false,
        k_synth_max: 8,
        quote_offsets_bps: vec![500],
        partial_crossing_fractions_bps: Vec::new(),
        debt_repayment_grid_bps: Vec::new(),
        synth_slot_fill_order: vec!["composite_drops".to_owned()],
        ..ScorerConfig::s1_standard(2)
    };
    let upkeep_reserve = seat_cfg
        .upkeep_amount
        .saturating_mul(i64::from(cfg.upkeep_cover_turns));
    let excess_amount: i64 = 1_000;
    obs.seats[0].refined[upkeep_idx] = upkeep_reserve.saturating_add(excess_amount);

    let new_upkeep_risk =
        mul_units_parity(excess_amount, obs.parities.refined_per_unit[upkeep_idx])
            .saturating_mul(SYNTH_DROP_UPKEEP_SELL_RISK_MULTIPLIER);
    let old_upkeep_risk = excess_amount.saturating_mul(SYNTH_DROP_UPKEEP_SELL_RISK_MULTIPLIER);
    let import_cost = town_import_cost_cred(
        per_run,
        obs.parities.raw_per_unit[usize::from(raw_idx)],
        obs.spec.town_import_premium_bps,
    )?;
    assert!(
        new_upkeep_risk < import_cost && import_cost < old_upkeep_risk,
        "expected {new_upkeep_risk} < {import_cost} < {old_upkeep_risk}"
    );

    let expected_imports: Vec<MarketAction> = seat_cfg.converters[0]
        .inputs
        .iter()
        .map(|&(in_g, req_amt)| MarketAction::TownImport {
            seat: 0,
            good: AssetId::Raw(in_g),
            amount: req_amt,
        })
        .collect();
    assert_eq!(
        expected_imports.len(),
        1,
        "expected single-input converter recipe so composite is strictly two legs"
    );
    let expected_sell_price = ((i128::from(obs.parities.refined_per_unit[upkeep_idx].max(1))
        * (league_core::BPS_SCALE + 500))
        / league_core::BPS_SCALE)
        .max(1) as i64;
    let expected_sell = MarketAction::PostOffer {
        seat: 0,
        order: OfferProposal {
            side: OfferSide::Sell,
            good: AssetId::Refined(seat_cfg.upkeep_good),
            amount: excess_amount,
            price_per_unit_cred: expected_sell_price,
            domain_id: None,
            is_hybrid: false,
        },
    };
    let expected_composite = vec![expected_imports[0].clone(), expected_sell.clone()];

    let cands = generate_arm_independent_candidates(&obs, &mandate, &cfg, 17)?;
    assert!(
        cands.iter().any(|c| {
            c.submission.market_actions == expected_composite
                && c.submission.pass_b_actions.is_empty()
                && c.submission.bundle == SealedOrderBundle::default()
        }),
        "expected exact two-leg composite seed [TownImport, PostOffer(Sell, upkeep_good)] in candidates"
    );
    assert!(
        cands.iter().any(|c| {
            c.submission.market_actions == vec![expected_sell.clone()]
                && c.submission.pass_b_actions.is_empty()
                && c.submission.bundle == SealedOrderBundle::default()
        }),
        "expected composite_drops to drop higher milli-CRED TownImport ({import_cost}) leaving exact [PostOffer(Sell, upkeep_good, amount={excess_amount})]"
    );
    assert!(
        !cands.iter().any(|c| {
            c.submission
                .market_actions
                .iter()
                .any(|a| matches!(a, MarketAction::TownImport { .. }))
                && !c
                    .submission
                    .market_actions
                    .iter()
                    .any(|a| matches!(a, MarketAction::PostOffer { .. }))
        }),
        "old raw-units formula would drop PostOffer first leaving TownImport alone"
    );
    Ok(())
}

#[test]
fn engine_cadences_and_charter_roles_pin_generator_rules() -> TestResult {
    // Exhaustive match without wildcard guarantees adding a CharterRole in league-core fails compilation here.
    let assert_charter_role_exhaustiveness = |r: CharterRole| match r {
        CharterRole::ClearinghouseBroker => "ClearinghouseBroker",
        CharterRole::OracleAuditor => "OracleAuditor",
        CharterRole::CustomsInspector => "CustomsInspector",
    };
    let all_roles = [
        CharterRole::ClearinghouseBroker,
        CharterRole::OracleAuditor,
        CharterRole::CustomsInspector,
    ];
    for r in all_roles {
        assert!(!assert_charter_role_exhaustiveness(r).is_empty());
    }

    let mut host = EvalHost::from_standard_setup(TableId(8265), 4, 1, 0)?;
    let cfg = ScorerConfig {
        k_synth_max: 64,
        ..ScorerConfig::default()
    };

    // Drive StandForOffice, council_vote, and all 3 CharterRole bids on EVERY turn 1..=12,
    // pinning % 4 (turns 4, 8, 12 vs % 2) and % 6 (turns 6, 12 vs % 2 and % 3).
    for expected_turn in 1_u8..=12_u8 {
        assert_eq!(host.turn(), expected_turn);
        let mut obs0 = host.observe_seat(Seat(0))?;
        obs0.seats[0].cred = 1_000_000;
        let m0 = *obs0.own_mandate().ok_or("missing seat-0 mandate")?;
        let cands = generate_arm_independent_candidates(&obs0, &m0, &cfg, 1)?;

        let gen_has_stand = cands.iter().any(|c| {
            c.submission
                .market_actions
                .iter()
                .any(|a| matches!(a, MarketAction::StandForOffice { .. }))
        });
        let gen_has_vote = cands
            .iter()
            .any(|c| c.submission.bundle.council_vote.is_some());
        let gen_has_any_charter = cands
            .iter()
            .any(|c| !c.submission.bundle.charter_bids.is_empty());

        let mut subs = BTreeMap::new();
        for (idx, role) in all_roles.into_iter().enumerate() {
            let s = idx as u8;
            let mut bundle = SealedOrderBundle::default();
            bundle.charter_bids.push(league_core::CharterBidOrder {
                bid_cred: SYNTH_CHARTER_BID_CRED,
                role,
            });
            let mut market_actions = Vec::new();
            if s == 0 {
                market_actions.push(MarketAction::StandForOffice {
                    seat: 0,
                    platform: Default::default(),
                });
                bundle.council_vote = Some(league_core::CouncilVoteBallot {
                    cartel_chair: 1,
                    trade_minister: 0,
                });
            }
            subs.insert(
                s,
                EvalActSubmission {
                    market_actions,
                    pass_b_actions: Vec::new(),
                    bundle,
                },
            );
        }

        let apply_out = host.apply_act_submissions(&subs)?;
        for s in 0..3_u8 {
            let rep_s = apply_out
                .seats
                .get(&s)
                .ok_or("missing apply report for bidder seat")?;
            assert!(
                rep_s.refused_market_actions.is_empty(),
                "turn {expected_turn} seat {s} market action refused: {:?}",
                rep_s.refused_market_actions
            );
            assert!(
                rep_s.refused_bundle_code.is_none(),
                "turn {expected_turn} seat {s} bundle refused: {:?}",
                rep_s.refused_bundle_code
            );
        }
        let rep = host.sweep_turn()?;

        let is_election = expected_turn.is_multiple_of(4);
        assert_eq!(
            rep.elected_officers.is_some(),
            is_election,
            "turn {expected_turn}: elected_officers mismatch"
        );
        assert_eq!(
            gen_has_stand, is_election,
            "turn {expected_turn}: generator StandForOffice cadence mismatch"
        );
        assert_eq!(
            gen_has_vote, is_election,
            "turn {expected_turn}: generator council_vote cadence mismatch"
        );

        let is_charter = expected_turn.is_multiple_of(6);
        assert_eq!(
            !rep.charters_awarded.is_empty(),
            is_charter,
            "turn {expected_turn}: charters_awarded cadence mismatch"
        );
        assert_eq!(
            gen_has_any_charter, is_charter,
            "turn {expected_turn}: generator charter_bids cadence mismatch"
        );
        if is_charter {
            assert_eq!(rep.charters_awarded.len(), all_roles.len());
            for role in all_roles {
                assert!(
                    cands.iter().any(|c| {
                        c.submission
                            .bundle
                            .charter_bids
                            .iter()
                            .any(|b| b.role == role)
                    }),
                    "turn {expected_turn}: expected synthetic generator CharterRole::{role:?}"
                );
                assert!(
                    rep.charters_awarded.iter().any(|&(r, _)| r == role),
                    "turn {expected_turn}: engine did not award {role:?}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn rollout_engine_error_branches_report_step_and_strip_paths() -> TestResult {
    let tmp = UniqueTmpDir::new("rollout_err_branches")?;
    let secret_dir = tmp.path().join("secret_rollout_err_dir_token");
    fs::create_dir_all(&secret_dir)?;

    let mut host = EvalHost::from_standard_setup(TableId(8266), 4, 1, 0)?
        .with_engine_commit(TEST_ENGINE_COMMIT)?;
    // Advance Turn 1 so Turn 2 has 1 logged turn and 1 live_state_digest in `.seal`.
    let mut t1_subs = BTreeMap::new();
    for s in 0..4_u8 {
        let obs = host.observe_seat(Seat(s))?;
        let mandate = *obs.own_mandate().ok_or("missing Turn-1 mandate")?;
        let dec = ClubBot::new(77_u64.wrapping_add(u64::from(s))).decide_full_turn(&obs, &mandate);
        t1_subs.insert(
            s,
            EvalActSubmission {
                market_actions: dec.pass_a_actions,
                pass_b_actions: dec.pass_b_actions,
                bundle: dec.bundle,
            },
        );
    }
    host.apply_act_submissions(&t1_subs)?;
    host.sweep_turn()?;

    let (save_path, seal_path) = host.write_pre_act_checkpoint(&secret_dir)?;
    let save_bytes = fs::read(&save_path)?;
    let seal_bytes = fs::read(&seal_path)?;
    let save_sha = sha256_hex(&save_bytes);
    let seal_sha = sha256_hex(&seal_bytes);
    let pair_sha = compute_pre_act_pair_sha256_hex(&save_bytes, &seal_bytes);

    let focal_obs = host.observe_seat(Seat(0))?;
    let focal_mandate = *focal_obs.own_mandate().ok_or("missing focal mandate")?;
    let scenarios = draw_scenarios(
        &focal_obs.spec,
        focal_mandate,
        0,
        55,
        &pair_sha,
        host.turn(),
        2,
    )?;
    let cfg = ScorerConfig::default();

    let mut valid_peer_subs = BTreeMap::new();
    for s in 1..4_u8 {
        valid_peer_subs.insert(s, EvalActSubmission::default());
    }

    // 1. Apply Turn-day error branch (`apply turn <day> submissions: ...`).
    let mut bad_peer_subs = valid_peer_subs.clone();
    bad_peer_subs.insert(9, EvalActSubmission::default());
    let apply_err = run_single_rollout_with_replay_log(
        &save_path,
        &seal_path,
        (&save_sha, &seal_sha, &pair_sha),
        0,
        &EvalActSubmission::default(),
        &bad_peer_subs,
        focal_mandate,
        &scenarios[0],
        PostureKind::Standard,
        &cfg,
    )
    .err()
    .ok_or("expected ENGINE_ERROR when apply_act_submissions fails on Turn day")?;
    assert_eq!(apply_err.code(), "ENGINE_ERROR");
    let apply_msg = apply_err.to_string();
    assert!(
        apply_msg.contains("apply turn 2 submissions:")
            && apply_msg.contains("InvalidSeat(9)")
            && apply_msg.contains("invalid seat index: 9"),
        "unexpected apply message: {apply_msg}"
    );
    assert!(
        !apply_msg.contains("secret_rollout_err_dir_token"),
        "dir path leaked in apply error: {apply_msg}"
    );

    // 2. Finalize error branch (`finalize_eval_season: ...`) driven by a Turn-3 seal whose
    // historical `live_state_digests[0]` mismatches Turn-1 replay while tip `live_state_digests[1]`
    // and canonical encoding pass single-read fork checks.
    host.apply_act_submissions(&BTreeMap::new())?;
    host.sweep_turn()?;
    let (save3_path, seal3_path) = host.write_pre_act_checkpoint(&secret_dir)?;
    let seal3_bytes = fs::read(&seal3_path)?;
    let needle = b"\"live_state_digests\":";
    let key_pos = seal3_bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .ok_or("missing live_state_digests key")?;
    let val_start = key_pos + needle.len();
    let mut depth = 0_usize;
    let mut val_end = val_start;
    while val_end < seal3_bytes.len() {
        match seal3_bytes[val_end] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    val_end += 1;
                    break;
                }
            }
            _ => {}
        }
        val_end += 1;
    }
    let mut digests: Vec<[u8; 32]> = serde_json::from_slice(&seal3_bytes[val_start..val_end])?;
    digests[0][0] ^= 1;
    let new_digests_bytes = serde_json::to_vec(&digests)?;
    let mut bad_seal_bytes = Vec::with_capacity(seal3_bytes.len() + new_digests_bytes.len());
    bad_seal_bytes.extend_from_slice(&seal3_bytes[..val_start]);
    bad_seal_bytes.extend_from_slice(&new_digests_bytes);
    bad_seal_bytes.extend_from_slice(&seal3_bytes[val_end..]);
    let bad_seal_sha = sha256_hex(&bad_seal_bytes);
    let save3_bytes = fs::read(&save3_path)?;
    let save3_sha = sha256_hex(&save3_bytes);
    let bad_pair_sha = compute_pre_act_pair_sha256_hex(&save3_bytes, &bad_seal_bytes);
    let bad_seal_path = secret_dir.join("bad_digest_turn3.seal");
    fs::write(&bad_seal_path, &bad_seal_bytes)?;

    let finalize_err = run_single_rollout_with_replay_log(
        &save3_path,
        &bad_seal_path,
        (&save3_sha, &bad_seal_sha, &bad_pair_sha),
        0,
        &EvalActSubmission::default(),
        &valid_peer_subs,
        focal_mandate,
        &scenarios[0],
        PostureKind::Standard,
        &cfg,
    )
    .err()
    .ok_or("expected ENGINE_ERROR when finalize_eval_season detects digest divergence")?;
    assert_eq!(finalize_err.code(), "ENGINE_ERROR");
    let finalize_msg = finalize_err.to_string();
    assert!(
        finalize_msg.contains("finalize_eval_season:")
            && finalize_msg.contains("post_state_digest diverged")
            && finalize_msg.contains("turn 1"),
        "unexpected finalize message: {finalize_msg}"
    );
    assert!(
        !finalize_msg.contains("secret_rollout_err_dir_token"),
        "dir path leaked in finalize error: {finalize_msg}"
    );

    // 3. Missing peer mandate error branch also carries step context without paths.
    let mut missing_mandate_scenario = scenarios[0].clone();
    missing_mandate_scenario.peer_mandates.remove(&1);
    let mandate_err = run_single_rollout_with_replay_log(
        &save_path,
        &seal_path,
        (&save_sha, &seal_sha, &pair_sha),
        0,
        &EvalActSubmission::default(),
        &valid_peer_subs,
        focal_mandate,
        &missing_mandate_scenario,
        PostureKind::Standard,
        &cfg,
    )
    .err()
    .ok_or("expected ENGINE_ERROR when peer mandate missing")?;
    assert_eq!(mandate_err.code(), "ENGINE_ERROR");
    let mandate_msg = mandate_err.to_string();
    assert!(mandate_msg.contains("missing peer mandate 1"));
    assert!(!mandate_msg.contains("secret_rollout_err_dir_token"));
    Ok(())
}

#[test]
fn s3a_freeze_helpers_and_cli_validation_cover_error_branches() -> TestResult {
    let tmp = UniqueTmpDir::new("freeze_helpers")?;
    let valid_hex = "a".repeat(64);
    let valid_view = ScorerFreezeBlindView {
        blind_inputs_manifest_sha256: valid_hex.clone(),
        scorer_config_sha256: valid_hex.clone(),
        scorer_tree_sha256: valid_hex.clone(),
        verified_checkpoints_sha256: valid_hex.clone(),
    };
    valid_view.validate()?;

    let mut bad_tree = valid_view.clone();
    bad_tree.scorer_tree_sha256 = "not_hex".to_owned();
    assert_eq!(
        bad_tree
            .validate()
            .err()
            .ok_or("expected bad scorer_tree_sha256")?
            .code(),
        "INVALID_HEX_DIGEST"
    );

    let mut bad_cfg = valid_view.clone();
    bad_cfg.scorer_config_sha256 = "z".repeat(64);
    assert_eq!(
        bad_cfg
            .validate()
            .err()
            .ok_or("expected bad scorer_config_sha256")?
            .code(),
        "INVALID_HEX_DIGEST"
    );

    let mut bad_manifest = valid_view;
    bad_manifest.blind_inputs_manifest_sha256 = "ab".to_owned();
    assert_eq!(
        bad_manifest
            .validate()
            .err()
            .ok_or("expected bad blind_inputs_manifest_sha256")?
            .code(),
        "INVALID_HEX_DIGEST"
    );

    let missing_blind = tmp.path.join("no_such_blind_dir");
    assert_eq!(
        compute_blind_inputs_manifest_sha256(&missing_blind)
            .err()
            .ok_or("expected NoBlindInputs on missing dir")?
            .code(),
        "NO_BLIND_INPUTS"
    );

    let empty_blind = tmp.path.join("empty_blind_dir");
    fs::create_dir_all(empty_blind.join("nested_dir"))?;
    fs::write(empty_blind.join("note.txt"), b"ignored")?;
    assert_eq!(
        compute_blind_inputs_manifest_sha256(&empty_blind)
            .err()
            .ok_or("expected NoBlindInputs on empty dir")?
            .code(),
        "NO_BLIND_INPUTS"
    );

    let missing_crate = tmp.path.join("no_such_crate");
    assert_eq!(
        compute_scorer_tree_sha256(&missing_crate)
            .err()
            .ok_or("expected Io error on missing crate dir")?
            .code(),
        "IO_ERROR"
    );

    let fake_crate = tmp.path.join("fake_crate");
    fs::create_dir_all(fake_crate.join("src"))?;
    fs::create_dir_all(fake_crate.join("target"))?;
    fs::create_dir_all(fake_crate.join(".git"))?;
    fs::write(fake_crate.join("src/lib.rs"), b"pub fn f() {}")?;
    fs::write(fake_crate.join("target/ignored.txt"), b"a")?;
    fs::write(fake_crate.join(".git/HEAD"), b"ref: refs/heads/main\n")?;
    let hash_a = compute_scorer_tree_sha256(&fake_crate)?;
    fs::write(fake_crate.join("target/ignored.txt"), b"changed_b")?;
    let hash_b = compute_scorer_tree_sha256(&fake_crate)?;
    assert_eq!(hash_a, hash_b, "target/ and .git/ must be excluded");

    // CLI `--freeze` / `--scorer-crate-dir` usage & error branches.
    let mut err_buf = Vec::new();
    assert_eq!(
        run_cli(["score", "--freeze"], &mut Vec::new(), &mut err_buf),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_buf).contains("USAGE_ERROR"));

    err_buf.clear();
    assert_eq!(
        run_cli(
            ["score", "--scorer-crate-dir"],
            &mut Vec::new(),
            &mut err_buf
        ),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_buf).contains("USAGE_ERROR"));

    let ckpts_dir = tmp.path.join("ckpts");
    fs::create_dir_all(&ckpts_dir)?;
    let cfg_path = tmp.path.join("cfg.json");
    let cfg = ScorerConfig {
        scenarios_m: 2,
        k_synth_max: 2,
        ..ScorerConfig::default()
    };
    fs::write(&cfg_path, to_sorted_pretty_json(&cfg)?)?;
    let blind_dir = tmp.path.join("blind_req");
    fs::create_dir_all(&blind_dir)?;
    fs::write(blind_dir.join("s1.json"), b"{}")?;
    let out_dir = tmp.path.join("scores_out");
    let missing_freeze = tmp.path.join("missing_freeze.json");
    err_buf.clear();
    assert_eq!(
        run_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--freeze",
                missing_freeze.to_str().ok_or("utf8")?,
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_buf
        ),
        EXIT_RUNTIME_ERROR
    );
    assert!(String::from_utf8_lossy(&err_buf).contains("IO_ERROR"));

    let bad_json_freeze = tmp.path.join("bad_json_freeze.json");
    fs::write(&bad_json_freeze, b"{not json")?;
    err_buf.clear();
    assert_eq!(
        run_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--freeze",
                bad_json_freeze.to_str().ok_or("utf8")?,
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_buf
        ),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_buf).contains("INVALID_JSON"));

    let bad_hex_freeze = tmp.path.join("bad_hex_freeze.json");
    fs::write(
        &bad_hex_freeze,
        to_sorted_pretty_json(&ScorerFreezeBlindView {
            blind_inputs_manifest_sha256: "bad".to_owned(),
            scorer_config_sha256: valid_hex.clone(),
            scorer_tree_sha256: valid_hex.clone(),
            verified_checkpoints_sha256: valid_hex,
        })?,
    )?;
    err_buf.clear();
    assert_eq!(
        run_cli(
            [
                "score",
                "--blind-inputs",
                blind_dir.to_str().ok_or("utf8")?,
                "--checkpoints",
                ckpts_dir.to_str().ok_or("utf8")?,
                "--config",
                cfg_path.to_str().ok_or("utf8")?,
                "--freeze",
                bad_hex_freeze.to_str().ok_or("utf8")?,
                "--out",
                out_dir.to_str().ok_or("utf8")?,
            ],
            &mut Vec::new(),
            &mut err_buf
        ),
        EXIT_USER_ERROR
    );
    assert!(String::from_utf8_lossy(&err_buf).contains("INVALID_HEX_DIGEST"));
    Ok(())
}

#[test]
fn blind_request_and_state_score_output_require_talk_extraction_counts() -> TestResult {
    let cfg = ScorerConfig {
        scenarios_m: 2,
        k_synth_max: 2,
        ..ScorerConfig::default()
    };
    let cand = build_candidate(EvalActSubmission::default(), true, true)?;
    let valid_hex = sha256_hex(b"seed");
    let req = BlindScoringRequest {
        candidates: vec![cand],
        focal_seat: 0,
        pre_act_pair_sha256: valid_hex.clone(),
        pre_act_save_sha256: valid_hex.clone(),
        pre_act_seal_sha256: valid_hex,
        schema_version: BLIND_SCORING_SCHEMA_VERSION,
        scorer_config_sha256: cfg.sha256_hex()?,
        scorer_seed: 7,
        state_id: "counts_required_day-01_seat-0".to_owned(),
        talk_commitments: Vec::new(),
        talk_extraction_counts: TalkExtractionCounts::default(),
    };
    let mut val = serde_json::to_value(&req)?;
    val.as_object_mut()
        .ok_or("expected JSON object")?
        .remove("talk_extraction_counts");
    let err = parse_blind_scoring_request_strict(&serde_json::to_vec(&val)?)
        .err()
        .ok_or("expected missing talk_extraction_counts error")?;
    assert_eq!(err.code(), "INVALID_JSON");
    assert_eq!(err.exit_code(), EXIT_USER_ERROR);
    assert!(
        err.to_string().contains("talk_extraction_counts"),
        "expected missing-field error to name `talk_extraction_counts`, got: {err}"
    );

    let outcome = ScenarioRolloutOutcome {
        mandate_bonus: 0,
        net_asset_equity: 10,
        project_equity: 0,
        total_equity: 10,
    };
    let out = score_outcomes(&req, &cfg, 1, &[vec![outcome; 2]])?;
    let mut out_val = serde_json::to_value(&out)?;
    out_val
        .as_object_mut()
        .ok_or("expected StateScoreOutput JSON object")?
        .remove("talk_extraction_counts");
    let out_err = serde_json::from_value::<StateScoreOutput>(out_val)
        .err()
        .ok_or("expected missing StateScoreOutput talk_extraction_counts error")?;
    assert!(
        out_err.to_string().contains("talk_extraction_counts"),
        "expected StateScoreOutput missing-field error to name `talk_extraction_counts`, got: {out_err}"
    );
    Ok(())
}

#[test]
fn talk_extraction_counts_from_sources_rejects_seen_overflow() -> TestResult {
    let neg = TalkSourceCounts {
        ambiguous: 0,
        matched: u64::MAX,
        seen: u64::MAX,
    };
    let talk = TalkSourceCounts {
        ambiguous: 0,
        matched: 1,
        seen: 1,
    };
    let err = TalkExtractionCounts::from_sources(neg, talk)
        .err()
        .ok_or("expected u64 overflow error")?;
    assert_eq!(err.code(), "INVALID_TALK_COMMITMENT");
    assert_eq!(err.exit_code(), EXIT_USER_ERROR);
    Ok(())
}

#[test]
fn manifest_and_scorer_tree_digests_cover_filter_skip_and_error_paths() -> TestResult {
    let tmp = UniqueTmpDir::new("digest_paths")?;

    let missing_dir = tmp.path.join("does_not_exist");
    let missing_manifest = compute_blind_inputs_manifest_sha256(&missing_dir)
        .err()
        .ok_or("expected missing blind inputs dir error")?;
    assert_eq!(missing_manifest.code(), "NO_BLIND_INPUTS");

    let empty_dir = tmp.path.join("empty_blind");
    fs::create_dir_all(&empty_dir)?;
    fs::write(empty_dir.join("notes.txt"), b"ignored")?;
    fs::create_dir_all(empty_dir.join("nested_dir.json"))?;
    let empty_manifest = compute_blind_inputs_manifest_sha256(&empty_dir)
        .err()
        .ok_or("expected empty blind inputs dir error")?;
    assert_eq!(empty_manifest.code(), "NO_BLIND_INPUTS");

    fs::write(empty_dir.join("alpha.json"), b"{\"a\":1}")?;
    let mut expected_entries = BTreeMap::new();
    expected_entries.insert("alpha.json".to_owned(), sha256_hex(b"{\"a\":1}"));
    assert_eq!(
        compute_blind_inputs_manifest_sha256(&empty_dir)?,
        compute_blind_inputs_manifest_sha256_from_entries(&expected_entries)
    );

    let missing_tree = compute_scorer_tree_sha256(&missing_dir)
        .err()
        .ok_or("expected missing crate dir error")?;
    assert_eq!(missing_tree.code(), "IO_ERROR");

    let tree_a = tmp.path.join("tree_a");
    let tree_b = tmp.path.join("tree_b");
    for root in [&tree_a, &tree_b] {
        fs::create_dir_all(root.join("src"))?;
        fs::write(root.join("Cargo.toml"), b"[package]\nname=\"demo\"\n")?;
        fs::write(root.join("src/lib.rs"), b"pub fn x() {}\n")?;
    }
    fs::create_dir_all(tree_a.join("target/debug"))?;
    fs::write(tree_a.join("target/debug/artifact.bin"), b"ignored")?;
    fs::create_dir_all(tree_a.join(".git/objects"))?;
    fs::write(tree_a.join(".git/HEAD"), b"ref: refs/heads/main\n")?;
    assert_eq!(
        compute_scorer_tree_sha256(&tree_a)?,
        compute_scorer_tree_sha256(&tree_b)?
    );
    Ok(())
}

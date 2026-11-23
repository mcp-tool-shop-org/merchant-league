//! Integration, contract, and blinding tests for `league-scorer`.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use league_core::{
    AssetId, ClubBot, EvalActSubmission, EvalHost, GradedMandateCard, MandateKind, MarketAction,
    PublicTableSpec, SealedOrderBundle, Seat, TableId, ToStructuredError as _, TurnPhase,
};
use league_scorer::cli::run_cli;
use league_scorer::{
    BLIND_SCORING_SCHEMA_VERSION, BlindCandidate, BlindScoringRequest, EXIT_OK, EXIT_RUNTIME_ERROR,
    EXIT_USER_ERROR, ExtractedTalkCommitment, ScenarioRolloutOutcome, ScorerConfig, ScorerError,
    StateScoreOutput, candidate_id_for_submission, compute_pre_act_pair_sha256_hex, draw_scenarios,
    execute_blind_request, is_valid_sha256_hex, run_single_rollout, score_outcomes, sha256_hex,
    to_sorted_compact_json, to_sorted_pretty_json, verify_checkpoint_pair,
};

type TestResult = Result<(), Box<dyn Error>>;

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
    let mut host = EvalHost::from_standard_setup(TableId(table_seed), 4, 1, 0)?;
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

    let err_default = match pilot_default.validate_for_s1() {
        Ok(()) => return Err("pilot default contains Active/Conservative postures".into()),
        Err(e) => e,
    };
    assert_eq!(err_default.code(), "POSTURE_NOT_IMPLEMENTED");
    assert_eq!(err_default.exit_code(), EXIT_USER_ERROR);
    assert!(err_default.to_string().contains("posture not built yet"));

    let mut cfg_bad_weight = ScorerConfig::s1_standard(4);
    cfg_bad_weight.posture_weights.conservative = 1;
    assert_eq!(
        cfg_bad_weight.validate_for_s1().err().map(|e| e.code()),
        Some("POSTURE_NOT_IMPLEMENTED")
    );

    let mut cfg_empty_sched = ScorerConfig::s1_standard(4);
    cfg_empty_sched.posture_schedule.clear();
    assert_eq!(
        cfg_empty_sched.validate_for_s1().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_zero_std = ScorerConfig::s1_standard(4);
    cfg_zero_std.posture_weights.standard = 0;
    assert_eq!(
        cfg_zero_std.validate_for_s1().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_odd_m = ScorerConfig::s1_standard(3);
    assert_eq!(
        cfg_odd_m.validate_for_s1().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );
    cfg_odd_m = ScorerConfig::s1_standard(0);
    assert_eq!(
        cfg_odd_m.validate_for_s1().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_bad_mode = ScorerConfig::s1_standard(4);
    cfg_bad_mode.mandate_stratification_mode = "other".to_owned();
    assert_eq!(
        cfg_bad_mode.validate_for_s1().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_bad_p = ScorerConfig::s1_standard(4);
    cfg_bad_p.p_honour = 1.5;
    assert_eq!(
        cfg_bad_p.validate_for_s1().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_bad_span = ScorerConfig::s1_standard(4);
    cfg_bad_span.prescreen_min_arm_indep_span = -1;
    assert_eq!(
        cfg_bad_span.validate_for_s1().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let mut cfg_bad_digest = ScorerConfig::s1_standard(4);
    cfg_bad_digest.talk_extractor_rules_digest = "xyz".to_owned();
    assert_eq!(
        cfg_bad_digest.validate_for_s1().err().map(|e| e.code()),
        Some("INVALID_HEX_DIGEST")
    );

    let mut cfg_bad_ver = ScorerConfig::s1_standard(4);
    cfg_bad_ver.talk_extractor_version = "  ".to_owned();
    assert_eq!(
        cfg_bad_ver.validate_for_s1().err().map(|e| e.code()),
        Some("INVALID_SCORER_CONFIG")
    );

    let cfg = ScorerConfig::s1_standard(4);
    assert!(cfg.validate_for_s1().is_ok());
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
    assert_eq!(
        bad_talk.validate(&cfg).err().map(|e| e.code()),
        Some("TALK_COMMITMENTS_NOT_IMPLEMENTED")
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
        scorer_config_sha256: cfg_sha,
        scorer_seed: 12345,
        state_id: "season8102_day-01_seat-0".to_owned(),
        talk_commitments: Vec::new(),
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

    for sub in [&c_passive.submission, &c_import.submission] {
        let rollout_res = run_single_rollout(
            &cas_save,
            &cas_seal,
            0,
            sub,
            &peer_subs_0,
            focal_mandate,
            &scenarios[0],
        )?;

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
        let (sb, _, _, _) = manual.finalize_eval_season()?;
        let row0 = sb
            .standings
            .iter()
            .find(|e| e.seat == 0)
            .ok_or("missing seat 0")?;
        assert_eq!(rollout_res.total_equity, row0.total_equity);
        assert_eq!(rollout_res.project_equity, row0.project_equity);
        assert_eq!(rollout_res.net_asset_equity, row0.net_asset_equity);
        assert_eq!(rollout_res.mandate_bonus, row0.mandate_bonus);
    }

    // Also run `execute_blind_request` and verify CLI two-pass byte-identity.
    let direct_out = execute_blind_request(&req, &cfg, &cas_save, &cas_seal)?;
    let blind_dir = tmp.path().join("blind_inputs");
    fs::create_dir_all(&blind_dir)?;
    fs::write(
        blind_dir.join(format!("{}.json", req.state_id)),
        to_sorted_compact_json(&req)?,
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
    assert_eq!(parsed, direct_out);

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
        scorer_config_sha256: cfg_sha,
        scorer_seed: 7,
        state_id: "state_8103".to_owned(),
        talk_commitments: Vec::new(),
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

    // Duplicate state_id across files refused.
    fs::write(blind_dir.join("01.json"), to_sorted_compact_json(&req)?)?;
    fs::write(blind_dir.join("02.json"), to_sorted_compact_json(&req)?)?;
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
    fs::remove_file(blind_dir.join("02.json"))?;

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
            ScorerError::PostureNotImplemented("active".into()),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::TalkCommitmentsNotImplemented(2),
            EXIT_USER_ERROR,
        ),
        (
            ScorerError::InvalidScorerConfig("odd M".into()),
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
        cfg_nan.validate_for_s1().err().map(|e| e.code()),
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

    assert_eq!(
        run_single_rollout(
            &tmp.path().join("missing.save.json"),
            &cas_seal,
            0,
            &c_passive.submission,
            &peer_subs,
            focal_mandate,
            &scenarios[0],
        )
        .err()
        .map(|e| e.code()),
        Some("ENGINE_ERROR")
    );

    assert_eq!(
        run_single_rollout(
            &cas_save,
            &cas_seal,
            4,
            &c_passive.submission,
            &peer_subs,
            focal_mandate,
            &scenarios[0],
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
            0,
            &c_passive.submission,
            &bad_peer_subs,
            focal_mandate,
            &scenarios[0],
        )
        .err()
        .map(|e| e.code()),
        Some("ENGINE_ERROR")
    );

    // 5-seat checkpoint refused by both run_single_rollout and execute_blind_request.
    let mut host5 = EvalHost::from_standard_setup(TableId(8109), 5, 1, 0)?;
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
            0,
            &c_passive.submission,
            &peer_subs,
            focal_mandate,
            &scenarios[0],
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
        scorer_config_sha256: cfg_sha,
        scorer_seed: 1,
        state_id: "fake_ckpt".to_owned(),
        talk_commitments: Vec::new(),
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
        scorer_config_sha256: cfg_sha,
        scorer_seed: 9,
        state_id: "state_8110".to_owned(),
        talk_commitments: Vec::new(),
    };

    let blind_dir = tmp.path().join("blind");
    fs::create_dir_all(&blind_dir)?;
    fs::write(blind_dir.join("notes.txt"), b"ignored")?;
    fs::create_dir_all(blind_dir.join("nested_dir"))?;
    fs::write(blind_dir.join("req.json"), to_sorted_compact_json(&req)?)?;
    let ckpts_dir = tmp.path().join("checkpoints");
    let out_dir = tmp.path().join("out");

    // Short flags `-h` and `-V`, plus stdout write failures.
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

    // Also run binary with `--help` and `--version` so CLI help/version paths work end-to-end.
    let bin = env!("CARGO_BIN_EXE_league-scorer");
    for arg in ["--help", "--version"] {
        let out = Command::new(bin).arg(arg).output()?;
        assert!(out.status.success(), "bin {arg}");
        assert!(!out.stdout.is_empty(), "bin {arg}");
    }
    Ok(())
}

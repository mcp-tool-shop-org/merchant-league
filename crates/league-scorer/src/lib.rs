//! Arm-blind, deterministic CPU decision scoring kernel (`crates/league-scorer`, S2).
//!
//! Reads only anonymized [`BlindScoringRequest`] JSON, frozen [`ScorerConfig`] parameters, and
//! content-addressed `day-DD-pre-act` checkpoint blobs. Zero dependency on `league-eval`,
//! `league-relay`, arm labels, or wording identifiers.

#![forbid(unsafe_code)]

pub mod candidates;
pub mod cli;
pub mod error;
pub mod postures;
pub mod rollout;
pub mod scenarios;
pub mod scoring;
pub mod types;

pub use candidates::{
    build_arm_independent_request, dedupe_candidates, generate_arm_independent_candidates,
};
pub use error::{EXIT_OK, EXIT_RUNTIME_ERROR, EXIT_USER_ERROR, ScorerError, sanitize_error_text};
pub use postures::{
    apply_talk_commitments, canonicalize_talk_commitments, decide_peer_turn,
    draw_honour_commitment, p_honour_to_ppm, posture_for_scenario, preview_submission,
    prune_refused_market_actions, submission_from_club_bot,
};
pub use rollout::{
    execute_blind_request, probe_checkpoint_sanitized, run_single_rollout,
    run_single_rollout_with_replay_log, verify_checkpoint_digests, verify_checkpoint_pair,
};
pub use scenarios::{ScenarioAssignment, draw_scenarios};
pub use scoring::score_outcomes;
pub use types::{
    BLIND_SCORING_SCHEMA_VERSION, BlindCandidate, BlindScoringRequest,
    COMPOSITE_GREEDY_RULES_DEFAULT, CandidateScoreOutput, DROP_TIE_BREAK_BY_CANDIDATE_ID,
    EVAL_TABLE_SEAT_COUNT, ExtractedTalkCommitment, HONOUR_PPM_SCALE, PostureKind, PostureWeights,
    PrescreenSummary, SYNTH_CHARTER_BID_CRED, SYNTH_CLAIM_MATURING_EXACT,
    SYNTH_CONVERTER_INPUT_DEFICIT_COVER, SYNTH_COUNCIL_SELF_NOMINATION,
    SYNTH_DROP_DEFAULT_ACTION_RISK_CRED, SYNTH_DROP_PROJECT_RISK_CRED,
    SYNTH_DROP_UPKEEP_SELL_RISK_MULTIPLIER, SYNTH_MAX_CONVERTER_BATCHES, SYNTH_MAX_PROJECT_BATCHES,
    SYNTH_PROJECT_RANK_BY_EQUITY, SYNTH_QUOTE_CRED_SHARE_BPS, SYNTH_QUOTE_MAX_UNITS,
    SYNTH_QUOTE_MIN_UNITS, SYNTH_SCALE_SEED_PER_AXIS, SYNTH_SCALE_SEED_POOL_MAX,
    SYNTH_SLOT_BUCKET_NAMES, ScenarioRolloutOutcome, ScorerConfig, ScorerFreezeBlindView,
    StateScoreOutput, TALK_EXTRACTOR_RULES_DIGEST_V2, TALK_EXTRACTOR_VERSION_V2,
    TalkExtractionCounts, TalkSourceCounts, WinnersCurseSummary, candidate_id_for_submission,
    compute_blind_inputs_manifest_sha256, compute_blind_inputs_manifest_sha256_from_entries,
    compute_pre_act_pair_sha256_hex, compute_scorer_tree_sha256, is_valid_sha256_hex,
    parse_blind_scoring_request_strict, sha256_hex, to_sorted_compact_json, to_sorted_pretty_json,
    validate_state_id, validate_talk_commitment,
};

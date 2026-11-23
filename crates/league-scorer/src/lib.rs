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
    draw_honour_commitment, posture_for_scenario, preview_submission, prune_refused_market_actions,
    submission_from_club_bot,
};
pub use rollout::{
    execute_blind_request, fork_checkpoint_sanitized, run_single_rollout,
    verify_and_fork_checkpoint, verify_checkpoint_digests, verify_checkpoint_pair,
};
pub use scenarios::{ScenarioAssignment, draw_scenarios};
pub use scoring::score_outcomes;
pub use types::{
    BLIND_SCORING_SCHEMA_VERSION, BlindCandidate, BlindScoringRequest, CandidateScoreOutput,
    DROP_TIE_BREAK_BY_CANDIDATE_ID, EVAL_TABLE_SEAT_COUNT, ExtractedTalkCommitment, PostureKind,
    PostureWeights, PrescreenSummary, SYNTH_CLAIM_MATURING_EXACT,
    SYNTH_CONVERTER_INPUT_DEFICIT_COVER, SYNTH_COUNCIL_SELF_NOMINATION,
    SYNTH_MAX_CONVERTER_BATCHES, SYNTH_MAX_PROJECT_BATCHES, SYNTH_PROJECT_RANK_BY_EQUITY,
    SYNTH_SLOT_BUCKET_NAMES, ScenarioRolloutOutcome, ScorerConfig, StateScoreOutput,
    WinnersCurseSummary, candidate_id_for_submission, compute_pre_act_pair_sha256_hex,
    is_valid_sha256_hex, parse_blind_scoring_request_strict, sha256_hex, to_sorted_compact_json,
    to_sorted_pretty_json, validate_state_id, validate_talk_commitment,
};

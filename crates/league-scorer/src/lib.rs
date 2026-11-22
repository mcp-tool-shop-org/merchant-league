//! Arm-blind, deterministic CPU decision scoring kernel (`crates/league-scorer`, S1).
//!
//! Reads only anonymized [`BlindScoringRequest`] JSON, frozen [`ScorerConfig`] parameters, and
//! content-addressed `day-DD-pre-act` checkpoint blobs. Zero dependency on `league-eval`,
//! `league-relay`, arm labels, or wording identifiers.

#![forbid(unsafe_code)]

pub mod cli;
pub mod error;
pub mod rollout;
pub mod scenarios;
pub mod scoring;
pub mod types;

pub use error::{EXIT_OK, EXIT_RUNTIME_ERROR, EXIT_USER_ERROR, ScorerError};
pub use rollout::{execute_blind_request, run_single_rollout, verify_checkpoint_pair};
pub use scenarios::{ScenarioAssignment, draw_scenarios};
pub use scoring::score_outcomes;
pub use types::{
    BLIND_SCORING_SCHEMA_VERSION, BlindCandidate, BlindScoringRequest, CandidateScoreOutput,
    DROP_TIE_BREAK_BY_CANDIDATE_ID, EVAL_TABLE_SEAT_COUNT, ExtractedTalkCommitment, PostureKind,
    PostureWeights, PrescreenSummary, SYNTH_CLAIM_MATURING_EXACT,
    SYNTH_CONVERTER_INPUT_DEFICIT_COVER, SYNTH_COUNCIL_SELF_NOMINATION,
    SYNTH_MAX_CONVERTER_BATCHES, SYNTH_MAX_PROJECT_BATCHES, SYNTH_PROJECT_RANK_BY_EQUITY,
    ScenarioRolloutOutcome, ScorerConfig, StateScoreOutput, WinnersCurseSummary,
    candidate_id_for_submission, compute_pre_act_pair_sha256_hex, is_valid_sha256_hex, sha256_hex,
    to_sorted_compact_json, to_sorted_pretty_json, validate_state_id,
};

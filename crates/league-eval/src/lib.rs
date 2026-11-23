//! Merchant League Evaluation Mode (`league-eval`) driver, frozen-state branch runner, and
//! deterministic state selector.
//!
//! All state transitions run exclusively through [`league_core::EvalHost`] (with
//! [`league_core::ClubBot`] for manual `file` seats) and prompt delivery runs through `league-relay`.

#![forbid(unsafe_code)]

pub mod blind_builder;
pub mod cli;
pub mod driver;
pub mod error;
pub mod freeze_gate;
pub mod records;
pub mod reply;
pub mod schema;
pub mod select;
pub mod serving;
pub mod talk_extract;
pub mod view;
pub mod wording;

pub use blind_builder::{
    ArmMapEntry, ArmMapStateFile, BuildBlindInputsOptions, BuildBlindInputsReport,
    run_build_blind_inputs,
};
pub use cli::{ErrorFormat, execute_cli, execute_cli_with_git_sha, main_entry};
pub use driver::{
    BranchOptions, NoopTransport, PlayOptions, persist_eval_game_state, run_branch_with_transport,
    run_branch_with_transport_and_git_sha, run_play_with_transport,
    run_play_with_transport_and_git_sha, seat_keeps_notes, validate_eval_seats_file,
};
pub use error::{EXIT_OK, EXIT_PARTIAL_SUCCESS, EXIT_RUNTIME_ERROR, EXIT_USER_ERROR, EvalError};
pub use freeze_gate::{
    CountDistribution, DESCRIPTIVE_ONLY_INFERENTIAL_STATUS, FreezeOptions, JoinOptions, JoinReport,
    JoinedDecisionRow, JoinedDecisionsFile, PerArmSummaryFile, PerArmSummaryRow,
    PerArmTalkCountsSummary, ScorerFreezeFile, run_freeze, run_join,
};
#[doc(hidden)]
pub use freeze_gate::{write_freeze_file_with_writer, write_freeze_file_with_writer_and_remover};
pub use records::{
    BranchArmOutcome, BranchManifest, BranchSummary, DecisionTokenTopLogprobs,
    DecisionTopLogprobEntry, EvalArm, EvalDecisionRecord, EvalManifest, EvalManifestSeat,
    EvalSetupFile, FailedRelayAttempt, PASS_B_LIMIT_STATEMENT, PREFLIGHT_WRONG_PART_STOP_THRESHOLD,
    RefusalCodeCounts, RefusedBundleRecord, TurnRegentAuditRecord, WrongPartCounter,
    count_bundle_legs, extract_wrapper_metadata, summarize_branch_outcomes,
};
pub use reply::{
    InvalidRecipientDiagnostic, ParsedActReply, ParsedEvalMessage, ParsedTalkReply,
    ReplyRefusalCode, parse_eval_act_reply, parse_eval_talk_reply,
};
pub use schema::{build_eval_response_format_config, build_eval_response_format_value};
pub use select::{
    SelectOptions, SelectedPreActState, SelectionList, TurnStratum, expand_home_path,
    expand_home_path_with_home, run_select, sanitize_home_path, sanitize_home_path_with_homes,
};
pub use serving::{
    CRATE_GIT_SHA, CacheSettingsInput, CacheSettingsRecord, CrateGitShaSource,
    EffectiveSamplingRecord, EngineKind, MachineManifestInput, MachineManifestRecord,
    MetadataProvenance, ModelManifestInput, ModelManifestRecord, ProvenanceTagged,
    ServerSettingsRecord, ServingManifestInput, ServingManifestRecord, WordingArmDigests,
    capture_serving_env_snapshot, capture_serving_env_snapshot_from,
    load_and_validate_serving_manifest, resolve_crate_git_sha_from, resolve_verified_crate_git_sha,
    resolve_verified_crate_git_sha_in, validate_crate_git_sha_against,
    validate_crate_git_sha_and_cleanliness, validate_serving_manifest_input,
};
pub use talk_extract::{
    TalkExtractionResult, TalkRulesV2Table, extract_talk_commitments_for_state,
    extract_talk_commitments_from_messages, extract_talk_commitments_with_counts_for_state,
    resolve_eval_seat_guilds, talk_rules_v2_digest, talk_rules_v2_digest_for,
};
pub use view::{
    EVAL_SEAT_VIEW_JSON_INDENT_SPACES, EVAL_SEAT_VIEW_SERIALIZER_NAME,
    build_act_negotiation_inputs, render_preview_state_json, render_seat_view_markdown,
};
pub use wording::{
    EVAL_MESSAGES_MAX_ITEMS, EVAL_REPLY_WORDING, EVAL_REPLY_WORDING_ARM, PINNED_WRAPPERS,
    REJECTED_WRAPPER_HASHES, check_wrapper_digest, eval_internal_slot_names, eval_wording_slot,
    resolve_pinned_wrapper,
};

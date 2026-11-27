//! Merchant League Evaluation Mode (`league-eval`) driver, frozen-state branch runner, and
//! deterministic state selector.
//!
//! All state transitions run exclusively through [`league_core::EvalHost`] (with
//! [`league_core::ClubBot`] for manual `file` seats) and prompt delivery runs through `league-relay`.

#![forbid(unsafe_code)]

pub mod baselines;
pub mod blind_builder;
pub mod cli;
pub mod driver;
pub mod error;
pub mod freeze_gate;
pub mod mechanisms;
pub mod records;
pub mod reply;
pub mod report;
pub mod schema;
pub mod seed_block;
pub mod select;
pub mod serving;
pub mod talk_extract;
pub mod verify_checkpoints;
pub mod view;
pub mod wording;

pub use baselines::{
    ALWAYS_DEFECT_ASK_MARKUP_BPS, ALWAYS_DEFECT_CHARTER_BID_CRED, ALWAYS_DEFECT_MAX_CANDIDATES,
    ALWAYS_DEFECT_MAX_CROSS_OFFERS, ALWAYS_DEFECT_TALK_WHOLE_UNITS, BASELINE_RNG_DOMAIN,
    EvalFileSeatModel, OwnToSeatHistory, PeerStance, RANDOM_LEGAL_CHARTER_BID_CRED,
    RANDOM_LEGAL_DEBT_FRACTIONS_BPS, RANDOM_LEGAL_IMPORT_AMOUNT, RANDOM_LEGAL_MAX_CANCEL_PER_KIND,
    RANDOM_LEGAL_MAX_CLAIM_CANDIDATES, RANDOM_LEGAL_MAX_CONVERTER_RUNS,
    RANDOM_LEGAL_MAX_MARKET_GRID_SIZE, RANDOM_LEGAL_MAX_PROJECT_BATCHES,
    RANDOM_LEGAL_OFFER_AMOUNTS, RANDOM_LEGAL_PRICE_BPS, TIT_FOR_TAT_PRICE_TOLERANCE_MILLI_CRED,
    compute_tit_for_tat_stances, decide_always_defect_act, decide_random_legal_act,
    decide_scripted_baseline_act, decide_scripted_baseline_talk, decide_tit_for_tat_act,
    decide_tit_for_tat_talk, derive_baseline_rng, encode_canonical_eval_reply,
    plan_always_defect_talk,
};
pub use blind_builder::{
    ArmMapEntry, ArmMapStateFile, BuildBlindInputsOptions, BuildBlindInputsReport,
    run_build_blind_inputs,
};
pub use cli::{ErrorFormat, execute_cli, execute_cli_with_git_sha, main_entry};
pub use driver::{
    BranchOptions, NoopTransport, PlayOptions, persist_eval_game_state,
    run_branch_with_communication_and_git_sha, run_branch_with_transport,
    run_branch_with_transport_and_git_sha, run_play_with_communication_and_git_sha,
    run_play_with_transport, run_play_with_transport_and_git_sha, seat_keeps_notes,
    validate_eval_seats_file, validate_eval_seats_for_communication,
};
pub use error::{EXIT_OK, EXIT_PARTIAL_SUCCESS, EXIT_RUNTIME_ERROR, EXIT_USER_ERROR, EvalError};
pub use freeze_gate::{
    CountDistribution, DESCRIPTIVE_ONLY_INFERENTIAL_STATUS, FreezeOptions, JoinOptions, JoinReport,
    JoinedDecisionRow, JoinedDecisionsFile, PerArmSummaryFile, PerArmSummaryRow,
    PerArmTalkCountsSummary, SCENARIO_RANDOMNESS_SCOPE_STATEMENT, ScorerFreezeFile, run_freeze,
    run_freeze_with_provenance, run_join,
};
#[doc(hidden)]
pub use freeze_gate::{write_freeze_file_with_writer, write_freeze_file_with_writer_and_remover};
pub use mechanisms::{
    DecisionMechanismMetrics, HTTP_REPLY_OK_STATUS, MECHANISM_DATA_LIMITS, PerArmMechanismSummary,
    REFUSED_BUNDLE_LEGS_MIN_COUNT, ReasoningLengthCounts, TALK_COMMITMENT_HONOUR_WINDOW_DAYS,
    TALK_COMMITMENT_MIN_FILL_BPS, TALK_COMMITMENT_PRICE_TOLERANCE_MILLI_CRED,
    TOOL_CALL_USED_MIN_COUNT, measure_reasoning_length_in_memory, summarize_arm_mechanisms,
};
pub use records::{
    BranchArmOutcome, BranchManifest, BranchSummary, CommunicationMode, DecisionTokenTopLogprobs,
    DecisionTopLogprobEntry, EvalArm, EvalDecisionRecord, EvalManifest, EvalManifestSeat,
    EvalSetupFile, FailedRelayAttempt, PASS_B_LIMIT_STATEMENT, PREFLIGHT_WRONG_PART_STOP_THRESHOLD,
    RefusalCodeCounts, RefusedBundleRecord, TurnRegentAuditRecord, WrongPartCounter,
    count_bundle_legs, extract_wrapper_metadata, summarize_branch_outcomes,
};
pub use reply::{
    InvalidRecipientDiagnostic, ParsedActReply, ParsedEvalMessage, ParsedTalkReply,
    ReplyRefusalCode, parse_eval_act_reply, parse_eval_act_reply_for_communication,
    parse_eval_silent_act_reply, parse_eval_talk_reply,
};
pub use report::{
    DEFAULT_REPORT_BOOTSTRAP_SAMPLES, DEFAULT_REPORT_BOOTSTRAP_SEED, DEFAULT_REPORT_CI_LEVEL,
    EXACT_SIGN_FLIP_MAX_SEEDS, EvalBranchReport, EvalReportResult, REPORT_BOOTSTRAP_RNG_DOMAIN,
    REPORT_SCHEMA_VERSION, REPORT_SIGN_FLIP_RNG_DOMAIN, ReportArmSummary, ReportOptions,
    ReportOutputFormat, ReportPairwiseContrast, SeedClusterDiff,
    compute_pairwise_contrast_from_seed_diffs, render_report_text, run_report,
};
pub use schema::{
    build_eval_response_format_config, build_eval_response_format_value,
    build_eval_silent_response_format_config, build_eval_silent_response_format_value,
};
pub use seed_block::{
    DEFAULT_SEED_BLOCK_RNG_SEED, SEED_BLOCK_RNG_DOMAIN, SEED_BLOCK_SCHEMA_VERSION,
    SeedBlockEntrant, SeedBlockEntry, SeedBlockGameSpec, SeedBlockManifest, SeedBlockOptions,
    build_seed_latin_square, run_seed_block, verify_latin_square_4x4,
};
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
#[doc(hidden)]
pub use talk_extract::extract_talk_commitments_from_messages_with_rules;
pub use talk_extract::{
    TalkExtractionResult, TalkRulesV2Table, extract_talk_commitments_for_state,
    extract_talk_commitments_from_messages, extract_talk_commitments_with_counts_for_state,
    resolve_eval_seat_guilds, talk_rules_v2_digest, talk_rules_v2_digest_for,
};
pub use verify_checkpoints::{
    VerifyCheckpointsOptions, VerifyCheckpointsReport, run_verify_checkpoints,
};
pub use view::{
    EVAL_SEAT_VIEW_JSON_INDENT_SPACES, EVAL_SEAT_VIEW_SERIALIZER_NAME,
    build_act_negotiation_inputs, render_preview_state_json, render_seat_view_markdown,
    render_seat_view_markdown_for_communication,
};
pub use wording::{
    EVAL_MESSAGES_MAX_ITEMS, EVAL_REPLY_WORDING, EVAL_REPLY_WORDING_ARM, EVAL_SILENT_REPLY_WORDING,
    EVAL_SILENT_REPLY_WORDING_ARM, PINNED_SILENT_WRAPPERS, PINNED_WRAPPERS,
    REJECTED_WRAPPER_HASHES, check_wrapper_digest, eval_internal_slot_names,
    eval_silent_wording_slot, eval_wording_slot, resolve_pinned_wrapper,
    resolve_pinned_wrapper_for_communication,
};

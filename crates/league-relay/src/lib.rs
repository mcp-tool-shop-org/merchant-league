//! Seat relay for Merchant League (`local` OpenAI-compatible VM servers and `openrouter`).
//!
//! `league-relay` drives automated `local` (OpenAI-compatible loopback / VM servers
//! such as vLLM, capped in GPU-seconds via `--cap-gpu-secs`) and `openrouter`
//! seats (gated behind [`LiveGate`], requiring `--live`, `--cap-credits`, and
//! `OPENROUTER_API_KEY`) one round (`talk` or `act`) at a time against local
//! `to-seat/`, `from-seat/`, and `relay/` directories while leaving `file` seats
//! to manual relay.

pub mod cli;
pub mod error;
pub mod relay;
pub mod seats;
pub mod secret;
pub mod transport;
pub mod wording;

pub use cli::{EXIT_OK, EXIT_PARTIAL_SUCCESS, EXIT_RUNTIME_ERROR, EXIT_USER_ERROR, ErrorFormat};
pub use error::{PreviewStateErrorKind, RelayError, StructuredError, ToStructuredError};
pub use relay::{
    CallLogEntry, GameSetupRecord, LocalReasoningBudgetKey, MemoryMode, NonPinnedModelFacingString,
    ResponseFormatConfig, Round, SavedDiscardedReplyRecord, SavedHttpReplyRecord,
    SavedToolRoundCall, SavedToolRoundRecord, SeatOutcome, SeatReport, SetupSeatRecord,
    TranscriptEntry, TurnConfig, TurnSummary, discarded_reply_file_path, http_reply_file_path,
    prepend_wrapper, run_turn, sha256_hex, tool_round_file_path, validate_day,
    validate_scratchpad_setup,
};
#[cfg(any(test, feature = "test-hooks"))]
pub use relay::{
    DurabilityEvent, set_fail_between_from_seat_and_ok_ledger_for_test,
    set_fail_from_seat_write_for_test, set_fail_pending_temp_write_for_test,
    set_fail_tool_round_write_for_test, take_durability_events_for_test,
    watch_durability_events_for_test,
};
pub use seats::{
    BackendKind, CostUnit, LOCAL_TIMEOUT_MARGIN_SECS, LocalSeat, OpenRouterSeat, ReasoningEffort,
    SeatBackend, SeatFile, SeatReasoningConfig, SeatSpec, normalize_allowed_vm_hosts,
    validate_local_apply_template_url, validate_local_base_url, validate_local_endpoints,
    validate_local_tokenize_url, validate_pinned_local_model, validate_pinned_openrouter_slug,
    validate_vm_host_allowance,
};
pub use secret::{
    ApiKey, FORBIDDEN_LOCAL_KEY_ENVS, LEAGUE_LOG_LEVEL_ENV, LiveGate, LocalGate, LogLevel,
    MAX_REDACTION_SCAN_BYTES, MAX_UNRECORDED_CHARGE_SCAN_BYTES, OPENROUTER_API_KEY_ENV,
    REDACTED_MARKER, cap_utf8_bytes, collect_env_redaction_keys, install_redacting_panic_hook,
    install_redacting_panic_hook_with_level, panic_hook_log_level, redact_and_truncate,
    redact_credentials_multi_with_limit_and_paths, redact_secrets, redact_secrets_multi_with_limit,
    redact_secrets_with_limit, set_panic_hook_log_level, truncate_after_redaction,
};
pub use transport::{
    LocalOpenAiTransport, MockDirTransport, OPENROUTER_CHAT_COMPLETIONS_URL, RawHttpResponse,
    Transport, TransportError, UreqTransport, map_local_ureq_error, map_ureq_error,
};
pub use wording::{
    ACTIVE_WORDING_ARM, AiPreviewWireOutput, CONTROL_ARM_WORDING, CONTROL_ARM_WORDING_R1,
    CONTROL_ARM_WORDING_R2, CONTROL_WORDING_ARM, CONTROL_WORDING_ARM_R1, CONTROL_WORDING_ARM_R2,
    EMC_OLDER_REPLY_EXCERPT_CHARS, EMC_VERBATIM_TURN_WINDOW, MAX_INTENT_HEADER_DEPTH,
    MAX_PREVIEW_TOOL_CALLS_PER_TURN, MAX_SCRATCHPAD_TOKENS, MAX_WHISPER_CHARS, MAX_WHISPER_WORDS,
    MODEL_FACING_WORDING, NegotiationInputs, PREVIEW_ARM_WORDING, PREVIEW_ARM_WORDING_R1,
    PREVIEW_ARM_WORDING_R2, PREVIEW_WORDING_ARM, PREVIEW_WORDING_ARM_R1, PREVIEW_WORDING_ARM_R2,
    PreviewToolMode, SanitizedWhisper, UntrustedWhisper, WordingEntry,
    build_control_turn_orders_tool_definition, build_preview_turn_orders_tool_definition,
    build_silent_control_turn_orders_tool_definition,
    build_silent_preview_turn_orders_tool_definition, canonical_tool_definition_sha256,
    cap_words_and_chars, control_r1_wording_slot, control_r1_wording_template,
    control_wording_slot, control_wording_template, escape_default_ignorable_code_points,
    execute_control_tool_call, execute_preview_tool_call, execute_silent_control_tool_call,
    execute_silent_preview_tool_call, extract_scratchpad_update, format_negotiation_block,
    format_sanitized_scratchpad_section, format_scratchpad_section, json_escape_untrusted_str,
    load_negotiation_inputs_if_present, parse_preview_tool_arguments,
    parse_silent_preview_tool_arguments, preview_r1_wording_slot, preview_r1_wording_template,
    preview_wording_slot, preview_wording_template, render_call_limit_reached_wire_json,
    render_control_call_limit_reached_wire_json, render_control_engine_refusal_wire_json,
    render_control_malformed_tool_call_wire_json, render_engine_refusal_wire_json,
    render_json_escaped_whisper_line, render_malformed_tool_call_wire_json,
    render_wording_slot_with, render_wording_template_with, sanitize_scratchpad_for_prompt,
    sanitize_whisper, split_negotiation_sections, strip_thinking_tags,
    try_render_wording_template_with, wording_slot, wording_template,
};
#[cfg(any(test, feature = "test-hooks"))]
pub use wording::{
    preview_seat_bundle_executions_for_table_for_test,
    reset_preview_seat_bundle_executions_for_table_for_test,
};

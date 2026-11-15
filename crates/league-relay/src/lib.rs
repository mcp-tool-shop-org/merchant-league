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

pub use error::RelayError;
pub use relay::{
    CallLogEntry, GameSetupRecord, MemoryMode, ResponseFormatConfig, Round, SeatOutcome,
    SeatReport, SetupSeatRecord, TranscriptEntry, TurnConfig, TurnSummary, prepend_wrapper,
    run_turn, sha256_hex, validate_day, validate_scratchpad_setup,
};
#[cfg(any(test, feature = "test-hooks"))]
pub use relay::{
    DurabilityEvent, set_fail_pending_temp_write_for_test, take_durability_events_for_test,
    watch_durability_events_for_test,
};
pub use seats::{
    BackendKind, CostUnit, LOCAL_TIMEOUT_MARGIN_SECS, LocalSeat, OpenRouterSeat, ReasoningEffort,
    SeatBackend, SeatFile, SeatReasoningConfig, SeatSpec, normalize_allowed_vm_hosts,
    validate_local_base_url, validate_local_endpoints, validate_local_tokenize_url,
    validate_pinned_local_model, validate_pinned_openrouter_slug, validate_vm_host_allowance,
};
pub use secret::{
    ApiKey, FORBIDDEN_LOCAL_KEY_ENVS, LiveGate, LocalGate, MAX_REDACTION_SCAN_BYTES,
    MAX_UNRECORDED_CHARGE_SCAN_BYTES, OPENROUTER_API_KEY_ENV, REDACTED_MARKER, cap_utf8_bytes,
    install_redacting_panic_hook, redact_and_truncate, redact_secrets,
    redact_secrets_multi_with_limit, redact_secrets_with_limit, truncate_after_redaction,
};
pub use transport::{
    LocalOpenAiTransport, MockDirTransport, OPENROUTER_CHAT_COMPLETIONS_URL, RawHttpResponse,
    Transport, TransportError, UreqTransport, map_local_ureq_error, map_ureq_error,
};
pub use wording::{
    ACTIVE_WORDING_ARM, EMC_OLDER_REPLY_EXCERPT_CHARS, EMC_VERBATIM_TURN_WINDOW,
    MAX_INTENT_HEADER_DEPTH, MAX_SCRATCHPAD_TOKENS, MAX_WHISPER_CHARS, MAX_WHISPER_WORDS,
    MODEL_FACING_WORDING, NegotiationInputs, SanitizedWhisper, UntrustedWhisper, WordingEntry,
    cap_words_and_chars, extract_scratchpad_update, format_negotiation_block,
    format_sanitized_scratchpad_section, format_scratchpad_section, json_escape_untrusted_str,
    load_negotiation_inputs_if_present, render_json_escaped_whisper_line, render_wording_slot_with,
    render_wording_template_with, sanitize_scratchpad_for_prompt, sanitize_whisper,
    split_negotiation_sections, strip_thinking_tags, try_render_wording_template_with,
    wording_slot, wording_template,
};

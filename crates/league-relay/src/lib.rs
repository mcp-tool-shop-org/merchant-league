//! Paper-test seat relay for Merchant League.
//!
//! `league-relay` drives automated `openrouter` seats one round (`talk` or
//! `act`) at a time against local `to-seat/`, `from-seat/`, and `relay/`
//! directories while leaving `file` seats (such as Claude subagents) to manual
//! relay. Live network calls are gated behind [`LiveGate`], which requires
//! `--live`, `--cap-credits`, and `OPENROUTER_API_KEY`.

pub mod cli;
pub mod error;
pub mod relay;
pub mod seats;
pub mod secret;
pub mod transport;

pub use error::RelayError;
pub use relay::{
    CallLogEntry, Round, SeatOutcome, SeatReport, TranscriptEntry, TurnConfig, TurnSummary,
    prepend_wrapper, run_turn, sha256_hex, validate_day,
};
pub use seats::{BackendKind, OpenRouterSeat, SeatBackend, SeatFile, SeatSpec};
pub use secret::{
    ApiKey, LiveGate, MAX_REDACTION_SCAN_BYTES, OPENROUTER_API_KEY_ENV, REDACTED_MARKER,
    cap_utf8_bytes, install_redacting_panic_hook, redact_and_truncate, redact_secrets,
    truncate_after_redaction,
};
pub use transport::{
    MockDirTransport, OPENROUTER_CHAT_COMPLETIONS_URL, RawHttpResponse, Transport, TransportError,
    UreqTransport,
};

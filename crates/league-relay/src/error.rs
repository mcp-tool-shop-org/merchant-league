//! Error types for `league-relay`.
//!
//! Every string carried inside [`RelayError`] is passed through
//! [`redact_secrets`](crate::secret::redact_secrets) before construction so an
//! API key can never leak through `Display` or `Debug`.

use std::fmt;

/// Errors returned when validating configuration or executing a relay turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayError {
    /// Command-line arguments were invalid or incomplete.
    Usage(String),
    /// A secret was passed on the command line or in a file instead of `OPENROUTER_API_KEY`.
    SecretInForbiddenPlace(String),
    /// Network calls were requested without `--live`.
    LiveFlagRequired,
    /// `--live` was passed without `OPENROUTER_API_KEY` set in the environment.
    MissingApiKey,
    /// `--live` was passed without a valid `--cap-credits <n>` value, or `--cap-credits` was invalid.
    InvalidCap(String),
    /// A seat is on its first turn and `--wrapper <path>` was not provided.
    MissingWrapper {
        /// Guild whose transcript had no prior turns.
        guild: String,
    },
    /// The seat configuration file could not be read or failed validation.
    SeatConfig(String),
    /// File I/O failed while reading inputs or writing outputs.
    Io(String),
    /// Existing `calls.jsonl` or transcript JSONL was malformed.
    CorruptLog(String),
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelayError::Usage(msg) => write!(f, "invalid arguments: {msg}"),
            RelayError::SecretInForbiddenPlace(msg) => write!(f, "forbidden key placement: {msg}"),
            RelayError::LiveFlagRequired => f.write_str(
                "refusing network call: pass --live (with OPENROUTER_API_KEY and --cap-credits <n>) or --mock-dir <dir>",
            ),
            RelayError::MissingApiKey => f.write_str(
                "refusing network call: OPENROUTER_API_KEY environment variable is not set or is empty",
            ),
            RelayError::InvalidCap(msg) => write!(f, "invalid credit cap: {msg}"),
            RelayError::MissingWrapper { guild } => write!(
                f,
                "seat '{guild}' has no prior transcript; --wrapper <path> is required on a seat's first turn"
            ),
            RelayError::SeatConfig(msg) => write!(f, "invalid seat file: {msg}"),
            RelayError::Io(msg) => write!(f, "file error: {msg}"),
            RelayError::CorruptLog(msg) => write!(f, "corrupt relay log: {msg}"),
        }
    }
}

impl std::error::Error for RelayError {}

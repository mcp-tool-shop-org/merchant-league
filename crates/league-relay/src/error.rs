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
    /// A secret was passed on the command line, in a file, or to a local transport.
    SecretInForbiddenPlace(String),
    /// Network calls to OpenRouter were requested without `--live`.
    LiveFlagRequired,
    /// `--live` was passed without `OPENROUTER_API_KEY` set in the environment.
    MissingApiKey,
    /// Required `--cap-credits <n>` / `--cap-gpu-secs <n>` was missing or invalid.
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
    /// Another `league-relay turn` already holds `<game-dir>/relay/turn.lock`, or `turn.lock` was deleted/replaced mid-turn.
    TurnLocked {
        /// Redacted path to `<game-dir>/relay/turn.lock`.
        lock_path: String,
        /// Optional redacted holder metadata (`pid=... started_utc=...`) or mid-turn compromise detail.
        holder: Option<String>,
    },
    /// Neither `relay/calls.jsonl` nor `relay/calls.pending.jsonl` could be written after a sent call;
    /// the pre-call reservation remains held in `relay/calls.reserved.jsonl` for the next run.
    UnrecordedCharge {
        /// Total credits (or `"<n> gpu_seconds"` for local seats) held across kept reservations and side-file lines.
        charged_credits: String,
        /// Redacted per-call summary (`seat`, `day`, `round`, `attempt`, `charged_credits`).
        entries_summary: String,
        /// Redacted underlying ledger and side-file I/O error details.
        detail: String,
    },
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
            RelayError::InvalidCap(msg) => write!(f, "invalid cap: {msg}"),
            RelayError::MissingWrapper { guild } => write!(
                f,
                "seat '{guild}' has no prior transcript; --wrapper <path> is required on a seat's first turn"
            ),
            RelayError::SeatConfig(msg) => write!(f, "invalid seat file: {msg}"),
            RelayError::Io(msg) => write!(f, "file error: {msg}"),
            RelayError::CorruptLog(msg) => write!(f, "corrupt relay log: {msg}"),
            RelayError::TurnLocked { lock_path, holder } => match holder {
                Some(h) if h.starts_with("compromised:") => {
                    let detail = h.trim_start_matches("compromised:").trim();
                    write!(
                        f,
                        "per-game turn lock {lock_path} was deleted or replaced while turn was running ({detail}); never delete relay/turn.lock"
                    )
                }
                Some(h) if !h.is_empty() => write!(
                    f,
                    "another league-relay turn is already running on this game directory (lock {lock_path} held by {h}); wait for it to finish"
                ),
                _ => write!(
                    f,
                    "another league-relay turn is already running on this game directory (lock {lock_path} is held by an active process); wait for it to finish"
                ),
            },
            RelayError::UnrecordedCharge {
                charged_credits,
                entries_summary,
                detail,
            } => {
                let is_gpu_secs = charged_credits.ends_with(" gpu_seconds");
                let amount_with_unit = if is_gpu_secs {
                    charged_credits.clone()
                } else {
                    format!("{charged_credits} credits")
                };
                let estimate_field = if is_gpu_secs {
                    "max_gpu_secs_per_call"
                } else {
                    "max_cost_per_call"
                };
                write!(
                    f,
                    "post-call charge {amount_with_unit} could not be written to ledger or pending side file ({entries_summary}); the charge is held as a reservation in relay/calls.reserved.jsonl that the next run counts at {estimate_field} — do not enter it into relay/calls.jsonl by hand or it will be counted twice: {detail}"
                )
            }
        }
    }
}

impl std::error::Error for RelayError {}

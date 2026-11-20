//! Error types and canonical [`StructuredError`] representation (`SHIP_GATE.md` `:26`) for
//! `league-relay`, re-exported from the `league-error` leaf crate.
//!
//! Every string carried inside [`RelayError`] and [`StructuredError`] is scrubbed before display
//! or JSON serialization so an API key, bearer credential, XRPL seed, or sealed game secret can
//! never leak through `Display`, `Debug`, `stderr`, or JSON error emission.

pub use league_error::{StructuredError, ToStructuredError};

use crate::secret::{
    ApiKey, MAX_REDACTION_SCAN_BYTES, MAX_UNRECORDED_CHARGE_SCAN_BYTES,
    redact_secrets_multi_with_limit,
};
use std::fmt;

/// Returns a copy of `se` whose `message` and optional `cause` have also been scrubbed against
/// relay-specific `keys` (`8+`-character substrings) and sealed game-secret rules up to
/// `max_bytes`.
#[must_use]
pub fn scrub_structured_error_with_keys(
    se: &StructuredError,
    keys: &[ApiKey],
    max_bytes: usize,
) -> StructuredError {
    let message = redact_secrets_multi_with_limit(&se.message, keys, max_bytes);
    let mut scrubbed = StructuredError::new(se.code, message, se.hint, se.retryable);
    if let Some(cause) = &se.cause {
        scrubbed = scrubbed.with_cause(redact_secrets_multi_with_limit(cause, keys, max_bytes));
    }
    scrubbed
}

/// Specific validation failure class for `--preview-tool on` preview-state inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStateErrorKind {
    /// Expected `<game-dir>/to-seat/day-<DD>-act-<Guild>.preview-state.json` is absent.
    Missing,
    /// Preview state file is unreadable, invalid JSON, or fails strict round-trip validation.
    Malformed,
    /// Preview state turn/day does not match `--day`.
    DayMismatch,
    /// Preview state seat or seat count does not match the asking seat or `seats.json`.
    SeatMismatch,
}

impl PreviewStateErrorKind {
    /// Stable uppercase error code for JSON/stderr reporting.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Missing => "PREVIEW_STATE_MISSING",
            Self::Malformed => "PREVIEW_STATE_MALFORMED",
            Self::DayMismatch => "PREVIEW_STATE_DAY_MISMATCH",
            Self::SeatMismatch => "PREVIEW_STATE_SEAT_MISMATCH",
        }
    }

    /// Operator remediation hint matching [`RelayError::remediation`].
    #[must_use]
    pub const fn remediation(self) -> &'static str {
        match self {
            Self::Missing => {
                "write the seat's per-turn `day-<DD>-act-<Guild>.preview-state.json` under `to-seat/` before running with `--preview-tool on`"
            }
            Self::Malformed => {
                "ensure the `.preview-state.json` file is exact JSON of `PublicPreviewState` from `SeatObservation::to_public_preview_state()`"
            }
            Self::DayMismatch => {
                "ensure the `.preview-state.json` file's `turn` and `baseline_predicates.turn` match `--day`"
            }
            Self::SeatMismatch => {
                "ensure the `.preview-state.json` file's `seat`, `baseline_predicates.seat`, and `spec.seat_count` match the target guild seat"
            }
        }
    }
}

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
    /// `<game-dir>/relay/setup.json` was written under an earlier relay schema or wording arm (`C-17`).
    IncompatibleSetup(String),
    /// Per-turn `.preview-state.json` input required by `--preview-tool on` on `act` failed validation.
    PreviewState {
        /// Specific failure class (missing, malformed, day mismatch, or seat mismatch).
        kind: PreviewStateErrorKind,
        /// Redacted diagnostic detail safe to print to stderr.
        detail: String,
    },
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

impl RelayError {
    /// Stable `SCREAMING_SNAKE_CASE` error identifier (`SHIP_GATE.md` `:26`).
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Usage(_) => "INVALID_USAGE",
            Self::SecretInForbiddenPlace(_) => "SECRET_IN_FORBIDDEN_PLACE",
            Self::LiveFlagRequired => "LIVE_FLAG_REQUIRED",
            Self::MissingApiKey => "MISSING_API_KEY",
            Self::InvalidCap(_) => "INVALID_CAP",
            Self::MissingWrapper { .. } => "MISSING_WRAPPER",
            Self::SeatConfig(_) => "INVALID_SEAT_CONFIG",
            Self::Io(_) => "RELAY_IO",
            Self::CorruptLog(_) => "CORRUPT_LOG",
            Self::IncompatibleSetup(_) => "INCOMPATIBLE_SETUP",
            Self::PreviewState { kind, .. } => kind.code(),
            Self::TurnLocked { .. } => "TURN_LOCKED",
            Self::UnrecordedCharge { .. } => "UNRECORDED_CHARGE",
        }
    }

    /// Actionable operator hint for this error variant.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Usage(_) => {
                "run 'league-relay --help' and check subcommand, flag names, and required options"
            }
            Self::SecretInForbiddenPlace(_) => {
                "supply OPENROUTER_API_KEY only in the environment for --live OpenRouter runs and unset all API keys for local or prebook-check runs"
            }
            Self::LiveFlagRequired => {
                "pass --live with --cap-credits <n> and OPENROUTER_API_KEY in the environment, or pass --mock-dir <dir> for offline runs"
            }
            Self::MissingApiKey => {
                "set a non-empty OPENROUTER_API_KEY in the process environment before passing --live"
            }
            Self::InvalidCap(_) => {
                "pass a finite non-negative --cap-credits <n> for OpenRouter seats or --cap-gpu-secs <n> for local seats"
            }
            Self::MissingWrapper { .. } => "pass --wrapper <path> when running a seat's first turn",
            Self::SeatConfig(_) => {
                "fix the seats JSON file so every seat has a unique guild, valid backend, pinned model, and valid bounds"
            }
            Self::Io(_) => {
                "verify --game-dir and input/output file paths exist and have read/write permissions"
            }
            Self::CorruptLog(_) => {
                "inspect relay/calls.jsonl, side files, setup.json, or seat transcripts for truncated or malformed JSON lines"
            }
            Self::IncompatibleSetup(_) => {
                "start a fresh --game-dir matching the active relay setup schema and wording arm"
            }
            Self::PreviewState { kind, .. } => kind.remediation(),
            Self::TurnLocked { .. } => {
                "wait for the active league-relay turn on this --game-dir to finish without deleting relay/turn.lock, then retry"
            }
            Self::UnrecordedCharge { .. } => {
                "restore write permissions under <game-dir>/relay/ so the next run reconciles relay/calls.reserved.jsonl automatically; do not copy the entry into relay/calls.jsonl by hand"
            }
        }
    }

    /// Whether retrying the identical operation without changing configuration might succeed.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::TurnLocked { .. })
    }

    /// CLI exit code for this error under the unified `0/1/2/3` convention (`SHIP_GATE.md` `:27`).
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_)
            | Self::SecretInForbiddenPlace(_)
            | Self::LiveFlagRequired
            | Self::MissingApiKey
            | Self::InvalidCap(_)
            | Self::MissingWrapper { .. }
            | Self::SeatConfig(_)
            | Self::IncompatibleSetup(_)
            | Self::PreviewState { .. } => 1,
            Self::Io(_)
            | Self::CorruptLog(_)
            | Self::TurnLocked { .. }
            | Self::UnrecordedCharge { .. } => 2,
        }
    }

    pub(crate) const fn redaction_scan_limit(&self) -> usize {
        match self {
            Self::UnrecordedCharge { .. } => MAX_UNRECORDED_CHARGE_SCAN_BYTES,
            _ => MAX_REDACTION_SCAN_BYTES,
        }
    }
}

impl ToStructuredError for RelayError {
    fn to_structured_error(&self) -> StructuredError {
        let limit = self.redaction_scan_limit();
        let message = redact_secrets_multi_with_limit(&self.to_string(), &[], limit);
        let base = StructuredError::new(self.code(), message, self.hint(), self.is_retryable());
        match self {
            Self::Io(detail) | Self::CorruptLog(detail) => {
                base.with_cause(redact_secrets_multi_with_limit(detail, &[], limit))
            }
            Self::TurnLocked {
                holder: Some(h), ..
            } if !h.is_empty() => base.with_cause(redact_secrets_multi_with_limit(h, &[], limit)),
            Self::UnrecordedCharge { detail, .. } if !detail.is_empty() => {
                base.with_cause(redact_secrets_multi_with_limit(detail, &[], limit))
            }
            _ => base,
        }
    }
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
            RelayError::IncompatibleSetup(msg) => write!(f, "incompatible game setup: {msg}"),
            RelayError::PreviewState { kind, detail } => {
                write!(f, "preview state error ({}): {detail}", kind.code())
            }
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

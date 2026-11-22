//! Structured errors and CLI exit codes for `league-eval`.

use std::fmt;

use league_core::{EngineError, StructuredError, ToStructuredError, scrub_secret_text};
use league_relay::RelayError;

pub use league_relay::{EXIT_OK, EXIT_PARTIAL_SUCCESS, EXIT_RUNTIME_ERROR, EXIT_USER_ERROR};

/// Domain errors returned by `league-eval` setup, parsing, turn execution, branching, and selection.
#[derive(Debug, Clone)]
pub enum EvalError {
    /// Invalid CLI arguments or conflicting flags (`exit 1`).
    Usage(String),
    /// Evaluation table seat count is not `4` (`D6` / `R9`, `exit 1`).
    InvalidSeatCount {
        /// Number of seats provided in `seats.json`.
        actual: usize,
    },
    /// A `file` backend seat in `seats.json` did not declare `"model": "ClubBot"` (`exit 1`).
    UnsupportedFileSeatModel {
        /// Guild name of the offending seat.
        guild: String,
        /// Configured `model` field, if any.
        model: Option<String>,
    },
    /// Required `--wrapper` file missing or unreadable (`exit 1`).
    MissingWrapper(String),
    /// Wrapper bytes are not in the pinned wrapper allow-list or `--wrapper-version` disagrees (`exit 1`).
    UnpinnedWrapper(String),
    /// Required `--serving-manifest <path>` missing when model seats are evaluated (`exit 1`).
    MissingServingManifest(String),
    /// Prefix caching is not proven disabled for all server cache layers (`exit 1`).
    PrefixCacheEnabled(String),
    /// Serving `parallel_slots` is not equal to `1` (`exit 1`).
    ParallelSlotsExceeded(String),
    /// Model file SHA-256 missing or invalid in serving manifest (`exit 1`).
    MissingModelSha256(String),
    /// GPU name missing in serving manifest (`exit 1`).
    MissingGpuName(String),
    /// GPU UUID / SHA-256 missing or invalid in serving manifest (`exit 1`).
    InvalidGpuUuidSha256(String),
    /// Build-time `crate_git_sha` is missing, `"unknown"`, or stale relative to git HEAD (`exit 2`).
    InvalidCrateGitSha(String),
    /// Operator serving manifest or env snapshot contains a secret or value that cannot be scrubbed (`exit 1`).
    UnsafeManifestValue(String),
    /// Server HTTP 200 reply omitted required prompt token IDs, reply token IDs, or top-k logprobs (`exit 2`).
    MissingReplyTelemetry(String),
    /// Reported refused actions exceed submitted actions (`exit 2`).
    InvalidActionCount(String),
    /// A Standing Regent intervened for a model-driven seat, voiding the game (`exit 2`).
    ModelSeatRegentIntervention(String),
    /// Unified response schema construction or relay setup validation failed (`exit 2`).
    SchemaValidation(String),
    /// Checkpoint files missing or invalid for branching (`exit 1`).
    CheckpointNotFound(String),
    /// Requested strata cannot be satisfied cleanly from available turns (`exit 1`).
    InsufficientTurnsForStrata(String),
    /// Malformed or unsupported `--strata` configuration (`exit 1`).
    InvalidStrata(String),
    /// Unknown focal seat guild requested for `branch` (`exit 1`).
    UnknownFocalSeat(String),
    /// A configured model seat sets `reasoning.exclude_from_transcript: true`, forbidden in Eval (`exit 1`).
    EvalThinkStrippingForbidden(String),
    /// A `relay/seats/` transcript file required by `branch` is missing when prior model rounds exist (`exit 1`).
    MissingBranchTranscript(String),
    /// A `checkpoints/` pre-act scratchpad file required by `branch` is missing (`exit 1`).
    MissingBranchScratchpad(String),
    /// Required pre-act `<game_dir>/to-seat/day-DD-act-<Guild>.negotiation.json` missing for `branch` (`exit 1`).
    MissingBranchNegotiation(String),
    /// Branch arm output directory already exists and is non-empty (`exit 1`).
    BranchArmDirNotEmpty(String),
    /// Branch `--out` directory already contains `branch_summary.json` (`exit 1`).
    BranchSummaryExists(String),
    /// Computed wrong-part refusal rate exceeded `[0.0, 1.0]` (`exit 2`).
    InvalidWrongPartRate(String),
    /// Required source `<game_dir>/setup.json` missing for `play` resume or `branch` check (`exit 1`).
    MissingSourceSetup(String),
    /// Wrapper, seats file digest, memory mode, arm, table, or seed disagrees with source `setup.json` (`exit 1`).
    IncompatibleEvalSetup(String),
    /// A `relay/from-seat/` reply file already exists for the current day on resume (`exit 1`).
    PartialTurnFromSeatExists(String),
    /// Game has zero automated model seats recorded in `manifest.json` for `select` (`exit 1`).
    NoModelSeatsForSelect(String),
    /// Underlying `EvalHost` / `league-core` error.
    Engine(EngineError),
    /// Underlying `league-relay` error.
    Relay(RelayError),
    /// Filesystem or JSON serialization/deserialization error (`exit 2`).
    Io(String),
}

impl EvalError {
    /// Stable `SCREAMING_SNAKE_CASE` error identifier (`SHIP_GATE.md` `:26`).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Usage(_) => "INVALID_USAGE",
            Self::InvalidSeatCount { .. } => "INVALID_EVAL_SEAT_COUNT",
            Self::UnsupportedFileSeatModel { .. } => "UNSUPPORTED_FILE_SEAT_MODEL",
            Self::MissingWrapper(_) => "MISSING_WRAPPER",
            Self::UnpinnedWrapper(_) => "UNPINNED_WRAPPER",
            Self::MissingServingManifest(_) => "MISSING_SERVING_MANIFEST",
            Self::PrefixCacheEnabled(_) => "PREFIX_CACHE_ENABLED",
            Self::ParallelSlotsExceeded(_) => "PARALLEL_SLOTS_EXCEEDED",
            Self::MissingModelSha256(_) => "MISSING_MODEL_SHA256",
            Self::MissingGpuName(_) => "MISSING_GPU_NAME",
            Self::InvalidGpuUuidSha256(_) => "INVALID_GPU_UUID_SHA256",
            Self::InvalidCrateGitSha(_) => "INVALID_CRATE_GIT_SHA",
            Self::UnsafeManifestValue(_) => "UNSAFE_MANIFEST_VALUE",
            Self::MissingReplyTelemetry(_) => "MISSING_REPLY_TELEMETRY",
            Self::InvalidActionCount(_) => "INVALID_ACTION_COUNT",
            Self::ModelSeatRegentIntervention(_) => "MODEL_SEAT_REGENT_INTERVENTION",
            Self::SchemaValidation(_) => "SCHEMA_VALIDATION_FAILED",
            Self::CheckpointNotFound(_) => "CHECKPOINT_NOT_FOUND",
            Self::InsufficientTurnsForStrata(_) => "INSUFFICIENT_TURNS_FOR_STRATA",
            Self::InvalidStrata(_) => "INVALID_STRATA",
            Self::UnknownFocalSeat(_) => "UNKNOWN_FOCAL_SEAT",
            Self::EvalThinkStrippingForbidden(_) => "EVAL_THINK_STRIPPING_FORBIDDEN",
            Self::MissingBranchTranscript(_) => "MISSING_BRANCH_TRANSCRIPT",
            Self::MissingBranchScratchpad(_) => "MISSING_BRANCH_SCRATCHPAD",
            Self::MissingBranchNegotiation(_) => "MISSING_BRANCH_NEGOTIATION",
            Self::BranchArmDirNotEmpty(_) => "BRANCH_ARM_DIR_NOT_EMPTY",
            Self::BranchSummaryExists(_) => "BRANCH_SUMMARY_EXISTS",
            Self::InvalidWrongPartRate(_) => "INVALID_WRONG_PART_RATE",
            Self::MissingSourceSetup(_) => "MISSING_SOURCE_SETUP",
            Self::IncompatibleEvalSetup(_) => "INCOMPATIBLE_EVAL_SETUP",
            Self::PartialTurnFromSeatExists(_) => "PARTIAL_TURN_FROM_SEAT_EXISTS",
            Self::NoModelSeatsForSelect(_) => "NO_MODEL_SEATS_FOR_SELECT",
            Self::Engine(err) => err.code(),
            Self::Relay(err) => err.code(),
            Self::Io(_) => "EVAL_IO",
        }
    }

    /// Unified CLI exit code (`0 / 1 / 2 / 3`, `SHIP_GATE.md` `:27`).
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_)
            | Self::InvalidSeatCount { .. }
            | Self::UnsupportedFileSeatModel { .. }
            | Self::MissingWrapper(_)
            | Self::UnpinnedWrapper(_)
            | Self::MissingServingManifest(_)
            | Self::PrefixCacheEnabled(_)
            | Self::ParallelSlotsExceeded(_)
            | Self::MissingModelSha256(_)
            | Self::MissingGpuName(_)
            | Self::InvalidGpuUuidSha256(_)
            | Self::UnsafeManifestValue(_)
            | Self::CheckpointNotFound(_)
            | Self::InsufficientTurnsForStrata(_)
            | Self::InvalidStrata(_)
            | Self::UnknownFocalSeat(_)
            | Self::EvalThinkStrippingForbidden(_)
            | Self::MissingBranchTranscript(_)
            | Self::MissingBranchScratchpad(_)
            | Self::MissingBranchNegotiation(_)
            | Self::BranchArmDirNotEmpty(_)
            | Self::BranchSummaryExists(_)
            | Self::MissingSourceSetup(_)
            | Self::IncompatibleEvalSetup(_)
            | Self::PartialTurnFromSeatExists(_)
            | Self::NoModelSeatsForSelect(_) => EXIT_USER_ERROR,
            Self::Relay(err) => err.exit_code(),
            Self::InvalidWrongPartRate(_)
            | Self::InvalidCrateGitSha(_)
            | Self::MissingReplyTelemetry(_)
            | Self::InvalidActionCount(_)
            | Self::ModelSeatRegentIntervention(_)
            | Self::SchemaValidation(_)
            | Self::Engine(_)
            | Self::Io(_) => EXIT_RUNTIME_ERROR,
        }
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(msg) => f.write_str(&scrub_secret_text(msg)),
            Self::InvalidSeatCount { actual } => write!(
                f,
                "INVALID_EVAL_SEAT_COUNT: Evaluation Mode requires exactly 4 seats (D6), got {actual}"
            ),
            Self::UnsupportedFileSeatModel { guild, model } => write!(
                f,
                "UNSUPPORTED_FILE_SEAT_MODEL: file seat '{guild}' has model {:?}; only \"ClubBot\" is permitted under league-eval",
                model
            ),
            Self::MissingWrapper(msg) => write!(f, "MISSING_WRAPPER: {}", scrub_secret_text(msg)),
            Self::UnpinnedWrapper(msg) => write!(f, "UNPINNED_WRAPPER: {}", scrub_secret_text(msg)),
            Self::MissingServingManifest(msg) => {
                write!(f, "MISSING_SERVING_MANIFEST: {}", scrub_secret_text(msg))
            }
            Self::PrefixCacheEnabled(msg) => {
                write!(f, "PREFIX_CACHE_ENABLED: {}", scrub_secret_text(msg))
            }
            Self::ParallelSlotsExceeded(msg) => {
                write!(f, "PARALLEL_SLOTS_EXCEEDED: {}", scrub_secret_text(msg))
            }
            Self::MissingModelSha256(msg) => {
                write!(f, "MISSING_MODEL_SHA256: {}", scrub_secret_text(msg))
            }
            Self::MissingGpuName(msg) => write!(f, "MISSING_GPU_NAME: {}", scrub_secret_text(msg)),
            Self::InvalidGpuUuidSha256(msg) => {
                write!(f, "INVALID_GPU_UUID_SHA256: {}", scrub_secret_text(msg))
            }
            Self::InvalidCrateGitSha(msg) => {
                write!(f, "INVALID_CRATE_GIT_SHA: {}", scrub_secret_text(msg))
            }
            Self::UnsafeManifestValue(msg) => {
                write!(f, "UNSAFE_MANIFEST_VALUE: {}", scrub_secret_text(msg))
            }
            Self::MissingReplyTelemetry(msg) => {
                write!(f, "MISSING_REPLY_TELEMETRY: {}", scrub_secret_text(msg))
            }
            Self::InvalidActionCount(msg) => {
                write!(f, "INVALID_ACTION_COUNT: {}", scrub_secret_text(msg))
            }
            Self::ModelSeatRegentIntervention(msg) => write!(
                f,
                "MODEL_SEAT_REGENT_INTERVENTION: {}",
                scrub_secret_text(msg)
            ),
            Self::SchemaValidation(msg) => {
                write!(f, "SCHEMA_VALIDATION_FAILED: {}", scrub_secret_text(msg))
            }
            Self::CheckpointNotFound(msg) => {
                write!(f, "CHECKPOINT_NOT_FOUND: {}", scrub_secret_text(msg))
            }
            Self::InsufficientTurnsForStrata(msg) => {
                write!(
                    f,
                    "INSUFFICIENT_TURNS_FOR_STRATA: {}",
                    scrub_secret_text(msg)
                )
            }
            Self::InvalidStrata(msg) => {
                write!(f, "INVALID_STRATA: {}", scrub_secret_text(msg))
            }
            Self::UnknownFocalSeat(msg) => {
                write!(f, "UNKNOWN_FOCAL_SEAT: {}", scrub_secret_text(msg))
            }
            Self::EvalThinkStrippingForbidden(msg) => {
                write!(
                    f,
                    "EVAL_THINK_STRIPPING_FORBIDDEN: {}",
                    scrub_secret_text(msg)
                )
            }
            Self::MissingBranchTranscript(msg) => {
                write!(f, "MISSING_BRANCH_TRANSCRIPT: {}", scrub_secret_text(msg))
            }
            Self::MissingBranchScratchpad(msg) => {
                write!(f, "MISSING_BRANCH_SCRATCHPAD: {}", scrub_secret_text(msg))
            }
            Self::MissingBranchNegotiation(msg) => {
                write!(f, "MISSING_BRANCH_NEGOTIATION: {}", scrub_secret_text(msg))
            }
            Self::BranchArmDirNotEmpty(msg) => {
                write!(f, "BRANCH_ARM_DIR_NOT_EMPTY: {}", scrub_secret_text(msg))
            }
            Self::BranchSummaryExists(msg) => {
                write!(f, "BRANCH_SUMMARY_EXISTS: {}", scrub_secret_text(msg))
            }
            Self::InvalidWrongPartRate(msg) => {
                write!(f, "INVALID_WRONG_PART_RATE: {}", scrub_secret_text(msg))
            }
            Self::MissingSourceSetup(msg) => {
                write!(f, "MISSING_SOURCE_SETUP: {}", scrub_secret_text(msg))
            }
            Self::IncompatibleEvalSetup(msg) => {
                write!(f, "INCOMPATIBLE_EVAL_SETUP: {}", scrub_secret_text(msg))
            }
            Self::PartialTurnFromSeatExists(msg) => {
                write!(
                    f,
                    "PARTIAL_TURN_FROM_SEAT_EXISTS: {}",
                    scrub_secret_text(msg)
                )
            }
            Self::NoModelSeatsForSelect(msg) => {
                write!(f, "NO_MODEL_SEATS_FOR_SELECT: {}", scrub_secret_text(msg))
            }
            Self::Engine(err) => write!(f, "{}", scrub_secret_text(&err.to_string())),
            Self::Relay(err) => write!(f, "{}", scrub_secret_text(&err.to_string())),
            Self::Io(msg) => f.write_str(&scrub_secret_text(msg)),
        }
    }
}

impl std::error::Error for EvalError {}

impl From<EngineError> for EvalError {
    fn from(value: EngineError) -> Self {
        Self::Engine(value)
    }
}

impl From<RelayError> for EvalError {
    fn from(value: RelayError) -> Self {
        Self::Relay(value)
    }
}

impl ToStructuredError for EvalError {
    fn to_structured_error(&self) -> StructuredError {
        match self {
            Self::Usage(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "pass valid `league-eval play|branch|select` flags or run `league-eval --help`",
                false,
            ),
            Self::InvalidSeatCount { .. } => StructuredError::new(
                self.code(),
                self.to_string(),
                "supply a seats.json with exactly 4 seats for Evaluation Mode runs (D6)",
                false,
            ),
            Self::UnsupportedFileSeatModel { .. } => StructuredError::new(
                self.code(),
                self.to_string(),
                "set `backend: \"file\", \"model\": \"ClubBot\"` for bot seats or `\"local\"` / `\"openrouter\"` for model seats",
                false,
            ),
            Self::MissingWrapper(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "pass `--wrapper <path>` pointing to a readable seat brief markdown file",
                false,
            ),
            Self::UnpinnedWrapper(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "pass the pinned `v2_seat_brief_2026_11_21_r3.md` wrapper and matching `--wrapper-version`",
                false,
            ),
            Self::MissingServingManifest(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "pass `--serving-manifest <path>` describing server, cache, GPU, and sampling settings",
                false,
            ),
            Self::PrefixCacheEnabled(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "disable per-request `cache_prompt`, host-RAM `--cache-ram 0`, and idle-slot caching `--no-cache-idle-slots` (or vLLM APC)",
                false,
            ),
            Self::ParallelSlotsExceeded(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "restart the inference server with `parallel_slots == 1`",
                false,
            ),
            Self::MissingModelSha256(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "record the 64-hex SHA-256 digest of the GGUF/weights file in `--serving-manifest`",
                false,
            ),
            Self::MissingGpuName(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "record the non-empty GPU device name in `--serving-manifest`",
                false,
            ),
            Self::InvalidGpuUuidSha256(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "supply a non-empty `gpu_uuid` (hashed at load time) or 64-hex `gpu_uuid_sha256`",
                false,
            ),
            Self::InvalidCrateGitSha(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "rebuild `league-eval` cleanly inside the checked-out git revision so `crate_git_sha` matches HEAD",
                false,
            ),
            Self::UnsafeManifestValue(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "remove raw GPU/MIG identifiers, control characters, or un-scrubbable secrets from serving metadata",
                false,
            ),
            Self::MissingReplyTelemetry(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "ensure the serving backend returns `prompt_token_ids`, reply token IDs, and top-k `logprobs.content` on HTTP 200 replies",
                false,
            ),
            Self::InvalidActionCount(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "inspect decision apply report counts: refused action count cannot exceed submitted action count",
                false,
            ),
            Self::ModelSeatRegentIntervention(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "this run is voided because a Standing Regent intervened on an automated model seat",
                false,
            ),
            Self::SchemaValidation(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "verify pinned response schema construction against league-relay validation",
                false,
            ),
            Self::CheckpointNotFound(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "ensure `checkpoints/day-DD-pre-act.{save.json,seal}` and `day-DD-act-recorded.seal` exist",
                false,
            ),
            Self::InsufficientTurnsForStrata(_) | Self::InvalidStrata(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "ensure each game ran enough turns and strata matches `2,2,2` summing to `--per-game`",
                false,
            ),
            Self::UnknownFocalSeat(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "pass `--seat <guild>` matching one of the 4 guilds recorded in the game's `seats.json`",
                false,
            ),
            Self::EvalThinkStrippingForbidden(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "set `reasoning.exclude_from_transcript: false` (or omit it) for every seat in Evaluation Mode",
                false,
            ),
            Self::MissingBranchTranscript(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "ensure `relay/seats/<Guild>.jsonl` exists in the source game directory before branching",
                false,
            ),
            Self::MissingBranchScratchpad(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "ensure `checkpoints/day-DD-pre-act.<Guild>.scratchpad.md` exists before branching note-keeping seats",
                false,
            ),
            Self::MissingBranchNegotiation(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "ensure `to-seat/day-DD-act-<Guild>.negotiation.json` exists alongside the pre-act checkpoint",
                false,
            ),
            Self::BranchArmDirNotEmpty(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "pass a fresh empty `--out` directory for `league-eval branch`",
                false,
            ),
            Self::BranchSummaryExists(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "pass a fresh `--out` directory that does not already contain `branch_summary.json`",
                false,
            ),
            Self::InvalidWrongPartRate(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "verify decision refusal counters do not exceed evaluated model round denominators",
                false,
            ),
            Self::MissingSourceSetup(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "ensure the source game directory contains `setup.json` written by `league-eval play`",
                false,
            ),
            Self::IncompatibleEvalSetup(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "pass matching `--wrapper`, `--seats`, `--memory`, `--arm`, `--table-id`, and `--seed`",
                false,
            ),
            Self::PartialTurnFromSeatExists(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "inspect or clear the incomplete `from-seat/day-DD-*` files before resuming, or start in a new `--game-dir`",
                false,
            ),
            Self::NoModelSeatsForSelect(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "select requires at least one automated model seat (`role = \"Model\"`) in `manifest.json`",
                false,
            ),
            Self::Engine(err) => err.to_structured_error(),
            Self::Relay(err) => err.to_structured_error(),
            Self::Io(_) => StructuredError::new(
                self.code(),
                self.to_string(),
                "check directory permissions and disk space in the game or output path",
                false,
            ),
        }
    }
}

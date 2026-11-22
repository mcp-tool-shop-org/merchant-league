//! Structured domain errors and exit codes for `league-scorer`.

use std::fmt;

use league_core::{StructuredError, ToStructuredError};

/// Process exit code for clean completion.
pub const EXIT_OK: i32 = 0;

/// Process exit code for CLI usage, configuration, or validation refusals.
pub const EXIT_USER_ERROR: i32 = 1;

/// Process exit code for runtime I/O, host replay, or internal execution failures.
pub const EXIT_RUNTIME_ERROR: i32 = 2;

/// Domain error returned by `league-scorer` validation, scenario draws, rollouts, scoring, and CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScorerError {
    /// Invalid CLI arguments or conflicting flags (`exit 1`).
    Usage(String),
    /// Unsupported `BlindScoringRequest::schema_version` (`exit 1`).
    UnsupportedSchemaVersion(u32),
    /// Invalid or unsafe `state_id` (`exit 1`).
    InvalidStateId(String),
    /// Malformed 64-character lowercase SHA-256 hex digest (`exit 1`).
    InvalidHexDigest {
        /// Name of the offending digest field.
        field: &'static str,
        /// Rejected digest value.
        value: String,
    },
    /// Focal seat index out of range (`exit 1`).
    InvalidFocalSeat {
        /// Supplied focal seat index.
        focal_seat: u8,
        /// Allowed upper bound (exclusive).
        seat_count: u8,
    },
    /// Candidate list in `BlindScoringRequest` is empty (`exit 1`).
    EmptyCandidates(String),
    /// Candidate list contains zero `in_span == true` entries (`exit 1`).
    NoInSpanCandidates(String),
    /// Candidate list is not strictly sorted ascending by `candidate_id` (`exit 1`).
    UnsortedCandidates {
        /// Previous `candidate_id` in slice order.
        prev: String,
        /// Current `candidate_id` that violated ascending order.
        current: String,
    },
    /// Duplicate `candidate_id` in `BlindScoringRequest::candidates` (`exit 1`).
    DuplicateCandidateId(String),
    /// `BlindCandidate::candidate_id` does not match canonical SHA-256 of its `submission` (`exit 1`).
    CandidateIdMismatch {
        /// Candidate index in request.
        index: usize,
        /// Declared `candidate_id`.
        declared: String,
        /// Computed SHA-256 of `submission`.
        actual: String,
    },
    /// `BlindScoringRequest::scorer_config_sha256` disagrees with loaded `ScorerConfig` (`exit 1`).
    ConfigHashMismatch {
        /// Hash declared on request.
        expected: String,
        /// Hash of loaded `ScorerConfig`.
        actual: String,
    },
    /// Posture requested in `ScorerConfig` is not implemented yet in S1 (`exit 1`).
    PostureNotImplemented(String),
    /// Non-empty `talk_commitments` requested before S2 (`exit 1`).
    TalkCommitmentsNotImplemented(usize),
    /// Invalid scenario count `M` or `ScorerConfig` invariant (`exit 1`).
    InvalidScorerConfig(String),
    /// JSON parse or serialization failure (`exit 1` for input configs/requests, `exit 2` internally).
    Json(String),
    /// Missing blind request files in `--blind-inputs` (`exit 1`).
    NoBlindInputs(String),
    /// Non-content-addressed layout or disallowed entry inside `--checkpoints` (`exit 1`).
    InvalidCheckpointsLayout(String),
    /// Referenced checkpoint blob missing from `--checkpoints` (`exit 1`).
    CheckpointNotFound(String),
    /// On-disk checkpoint bytes failed `save`, `seal`, or `pair` SHA-256 verification (`exit 1`).
    CheckpointHashMismatch(String),
    /// Checkpoint table seat count is incompatible with Evaluation Mode (`exit 1`).
    InvalidCheckpointState(String),
    /// Focal seat's dealt mandate was not present in `spec.mandate_deck` (`exit 2`).
    FocalMandateNotInDeck,
    /// Remaining undealt mandate pool is smaller than `seat_count - 1` (`exit 2`).
    InsufficientMandatePool {
        /// Number of remaining entries in undealt pool.
        pool_len: usize,
        /// Number of non-focal peers requiring distinct mandates.
        needed: usize,
    },
    /// Underlying `league-core` engine failure during fork/step/finalize (`exit 2`).
    Engine(String),
    /// Filesystem or stdout/stderr I/O failure (`exit 2`).
    Io(String),
}

impl ScorerError {
    /// Stable uppercase machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Usage(_) => "USAGE_ERROR",
            Self::UnsupportedSchemaVersion(_) => "UNSUPPORTED_SCHEMA_VERSION",
            Self::InvalidStateId(_) => "INVALID_STATE_ID",
            Self::InvalidHexDigest { .. } => "INVALID_HEX_DIGEST",
            Self::InvalidFocalSeat { .. } => "INVALID_FOCAL_SEAT",
            Self::EmptyCandidates(_) => "EMPTY_CANDIDATES",
            Self::NoInSpanCandidates(_) => "NO_IN_SPAN_CANDIDATES",
            Self::UnsortedCandidates { .. } => "UNSORTED_CANDIDATES",
            Self::DuplicateCandidateId(_) => "DUPLICATE_CANDIDATE_ID",
            Self::CandidateIdMismatch { .. } => "CANDIDATE_ID_MISMATCH",
            Self::ConfigHashMismatch { .. } => "CONFIG_HASH_MISMATCH",
            Self::PostureNotImplemented(_) => "POSTURE_NOT_IMPLEMENTED",
            Self::TalkCommitmentsNotImplemented(_) => "TALK_COMMITMENTS_NOT_IMPLEMENTED",
            Self::InvalidScorerConfig(_) => "INVALID_SCORER_CONFIG",
            Self::Json(_) => "INVALID_JSON",
            Self::NoBlindInputs(_) => "NO_BLIND_INPUTS",
            Self::InvalidCheckpointsLayout(_) => "INVALID_CHECKPOINTS_LAYOUT",
            Self::CheckpointNotFound(_) => "CHECKPOINT_NOT_FOUND",
            Self::CheckpointHashMismatch(_) => "CHECKPOINT_HASH_MISMATCH",
            Self::InvalidCheckpointState(_) => "INVALID_CHECKPOINT_STATE",
            Self::FocalMandateNotInDeck => "FOCAL_MANDATE_NOT_IN_DECK",
            Self::InsufficientMandatePool { .. } => "INSUFFICIENT_MANDATE_POOL",
            Self::Engine(_) => "ENGINE_ERROR",
            Self::Io(_) => "IO_ERROR",
        }
    }

    /// Process exit code aligned with repository CLI conventions (`1` user/validation, `2` runtime).
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_)
            | Self::UnsupportedSchemaVersion(_)
            | Self::InvalidStateId(_)
            | Self::InvalidHexDigest { .. }
            | Self::InvalidFocalSeat { .. }
            | Self::EmptyCandidates(_)
            | Self::NoInSpanCandidates(_)
            | Self::UnsortedCandidates { .. }
            | Self::DuplicateCandidateId(_)
            | Self::CandidateIdMismatch { .. }
            | Self::ConfigHashMismatch { .. }
            | Self::PostureNotImplemented(_)
            | Self::TalkCommitmentsNotImplemented(_)
            | Self::InvalidScorerConfig(_)
            | Self::Json(_)
            | Self::NoBlindInputs(_)
            | Self::InvalidCheckpointsLayout(_)
            | Self::CheckpointNotFound(_)
            | Self::CheckpointHashMismatch(_)
            | Self::InvalidCheckpointState(_) => EXIT_USER_ERROR,
            Self::FocalMandateNotInDeck
            | Self::InsufficientMandatePool { .. }
            | Self::Engine(_)
            | Self::Io(_) => EXIT_RUNTIME_ERROR,
        }
    }

    /// Human-readable remediation hint matching [`ToStructuredError`].
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Usage(_) => "Run `league-scorer --help` and supply all required score flags.",
            Self::UnsupportedSchemaVersion(_) => {
                "Generate blind scoring requests with schema_version = 1."
            }
            Self::InvalidStateId(_) => {
                "Use a non-empty ASCII state_id free of path separators, colons, or whitespace."
            }
            Self::InvalidHexDigest { .. } => {
                "Provide a 64-character lowercase hexadecimal SHA-256 digest."
            }
            Self::InvalidFocalSeat { .. } => {
                "Set focal_seat within 0..seat_count (0..4 for Evaluation Mode tables)."
            }
            Self::EmptyCandidates(_) => {
                "Include at least one candidate (passive baseline plus span entries)."
            }
            Self::NoInSpanCandidates(_) => {
                "Mark at least one candidate with in_span: true for [v_min, v_max] normalization."
            }
            Self::UnsortedCandidates { .. } | Self::DuplicateCandidateId(_) => {
                "Deduplicate candidates and sort ascending by canonical SHA-256 candidate_id."
            }
            Self::CandidateIdMismatch { .. } => {
                "Recompute candidate_id as sha256_hex(serde_json::to_vec(&submission))."
            }
            Self::ConfigHashMismatch { .. } => {
                "Pass the matching ScorerConfig JSON or rebuild requests against the frozen config."
            }
            Self::PostureNotImplemented(_) => {
                "Configure all-Standard posture weights/schedule for S1 scoring."
            }
            Self::TalkCommitmentsNotImplemented(_) => {
                "Pass an empty talk_commitments array in S1 scoring requests."
            }
            Self::InvalidScorerConfig(_) => {
                "Verify scenarios_m is an even integer >= 2 and all config fields are valid."
            }
            Self::Json(_) => "Ensure request and config JSON match the strict S1 schemas.",
            Self::NoBlindInputs(_) => {
                "Provide at least one *.json request file inside --blind-inputs."
            }
            Self::InvalidCheckpointsLayout(_) => {
                "Place only content-addressed <64-hex>.save.json and <64-hex>.seal files in --checkpoints."
            }
            Self::CheckpointNotFound(_) => {
                "Copy the referenced <sha256>.save.json and <sha256>.seal files into --checkpoints."
            }
            Self::CheckpointHashMismatch(_) => {
                "Re-export uncorrupted checkpoint files matching pre_act_{save,seal,pair}_sha256."
            }
            Self::InvalidCheckpointState(_) => {
                "Supply a 4-seat Evaluation Mode day-DD-pre-act checkpoint."
            }
            Self::FocalMandateNotInDeck | Self::InsufficientMandatePool { .. } => {
                "Verify checkpoint spec.mandate_deck includes all dealt mandates and enough undealt entries."
            }
            Self::Engine(_) => "Inspect the checkpoint integrity and EvalHost phase prerequisites.",
            Self::Io(_) => "Verify filesystem paths and permissions for inputs and outputs.",
        }
    }
}

impl fmt::Display for ScorerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.code();
        match self {
            Self::Usage(msg)
            | Self::InvalidStateId(msg)
            | Self::EmptyCandidates(msg)
            | Self::NoInSpanCandidates(msg)
            | Self::DuplicateCandidateId(msg)
            | Self::PostureNotImplemented(msg)
            | Self::InvalidScorerConfig(msg)
            | Self::Json(msg)
            | Self::NoBlindInputs(msg)
            | Self::InvalidCheckpointsLayout(msg)
            | Self::CheckpointNotFound(msg)
            | Self::CheckpointHashMismatch(msg)
            | Self::InvalidCheckpointState(msg)
            | Self::Engine(msg)
            | Self::Io(msg) => write!(f, "{code}: {msg}"),
            Self::UnsupportedSchemaVersion(ver) => {
                write!(
                    f,
                    "{code}: unsupported blind schema_version {ver}; expected 1"
                )
            }
            Self::InvalidHexDigest { field, value } => {
                write!(
                    f,
                    "{code}: malformed 64-hex SHA-256 in `{field}`: `{value}`"
                )
            }
            Self::InvalidFocalSeat {
                focal_seat,
                seat_count,
            } => write!(
                f,
                "{code}: focal_seat {focal_seat} out of range 0..{seat_count}"
            ),
            Self::UnsortedCandidates { prev, current } => write!(
                f,
                "{code}: candidates must be strictly sorted ascending (`{prev}` before `{current}`)"
            ),
            Self::CandidateIdMismatch {
                index,
                declared,
                actual,
            } => write!(
                f,
                "{code}: candidate[{index}] id `{declared}` != canonical submission digest `{actual}`"
            ),
            Self::ConfigHashMismatch { expected, actual } => write!(
                f,
                "{code}: request scorer_config_sha256 `{expected}` != loaded config `{actual}`"
            ),
            Self::TalkCommitmentsNotImplemented(len) => write!(
                f,
                "{code}: posture/compliance feature not built yet (received {len} talk_commitments)"
            ),
            Self::FocalMandateNotInDeck => write!(
                f,
                "{code}: focal seat mandate was not found in checkpoint spec.mandate_deck"
            ),
            Self::InsufficientMandatePool { pool_len, needed } => write!(
                f,
                "{code}: undealt mandate pool has {pool_len} entries, need {needed}"
            ),
        }
    }
}

impl std::error::Error for ScorerError {}

impl ToStructuredError for ScorerError {
    fn to_structured_error(&self) -> StructuredError {
        StructuredError::new(self.code(), self.to_string(), self.hint(), false)
    }
}

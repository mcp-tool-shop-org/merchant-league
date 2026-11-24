//! Structured domain errors and exit codes for `league-scorer`.

use std::fmt;
use std::path::Path;

use league_core::eval::host::checkpoint_verify::CheckpointVerifyError;
use league_core::{StructuredError, ToStructuredError};

/// Process exit code for clean completion.
pub const EXIT_OK: i32 = 0;

/// Process exit code for CLI usage, configuration, or validation refusals.
pub const EXIT_USER_ERROR: i32 = 1;

/// Process exit code for runtime I/O, host replay, or internal execution failures.
pub const EXIT_RUNTIME_ERROR: i32 = 2;

/// Strips directory prefixes of known paths from error strings so full filesystem paths never appear.
#[must_use]
pub fn sanitize_error_text(msg: &str, sensitive_paths: &[&Path]) -> String {
    let mut cleaned = msg.to_owned();
    for path in sensitive_paths {
        let base = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "<file>".to_owned());
        let full = path.to_string_lossy().into_owned();
        if !full.is_empty() {
            cleaned = cleaned.replace(&full, &base);
            let fwd = full.replace('\\', "/");
            cleaned = cleaned.replace(&fwd, &base);
        }
        if let Some(parent) = path.parent() {
            let parent_str = parent.to_string_lossy().into_owned();
            if !parent_str.is_empty() {
                for sep in ["/", "\\"] {
                    let with_sep = format!("{parent_str}{sep}");
                    let with_fwd_sep = format!("{}{sep}", parent_str.replace('\\', "/"));
                    cleaned = cleaned.replace(&with_sep, "");
                    cleaned = cleaned.replace(&with_fwd_sep, "");
                }
                cleaned = cleaned.replace(&parent_str, "<dir>");
                let fwd_parent = parent_str.replace('\\', "/");
                cleaned = cleaned.replace(&fwd_parent, "<dir>");
            }
        }
    }
    cleaned
}

/// Domain error returned by `league-scorer` validation, candidate generation, rollouts, scoring, and CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScorerError {
    /// Invalid CLI arguments or conflicting flags (`exit 1`).
    Usage(String),
    /// Unsupported `BlindScoringRequest::schema_version` (`exit 1`).
    UnsupportedSchemaVersion(u32),
    /// Invalid, unsafe, or colliding `state_id` (`exit 1`).
    InvalidStateId(String),
    /// Request JSON filename stem does not equal `BlindScoringRequest::state_id` (`exit 1`).
    StateIdFilenameMismatch(String),
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
    /// Candidate `submission` payload contains unknown or non-canonical JSON fields (`exit 1`).
    UnknownSubmissionField(String),
    /// `BlindScoringRequest::scorer_config_sha256` disagrees with loaded `ScorerConfig` (`exit 1`).
    ConfigHashMismatch {
        /// Hash declared on request.
        expected: String,
        /// Hash of loaded `ScorerConfig`.
        actual: String,
    },
    /// `--freeze` `scorer_config_sha256` disagrees with loaded `ScorerConfig` (`exit 1`).
    FreezeConfigMismatch(String),
    /// `--freeze` `scorer_tree_sha256` disagrees with `crates/league-scorer/**` (`exit 1`).
    FreezeTreeMismatch(String),
    /// `--freeze` `blind_inputs_manifest_sha256` disagrees with `--blind-inputs` (`exit 1`).
    FreezeManifestMismatch(String),
    /// Required `--verified-checkpoints` flag or `verified_checkpoints_sha256` freeze binding missing (`exit 1`).
    MissingVerifiedCheckpoints(String),
    /// `--verified-checkpoints` SHA-256 disagrees with `--freeze` `verified_checkpoints_sha256` (`exit 1`).
    FreezeVerifiedCheckpointsMismatch(String),
    /// Typed causal/canonical checkpoint verification failure (`exit 1` or `exit 2`).
    CheckpointVerify(CheckpointVerifyError),
    /// Invalid `ExtractedTalkCommitment` in `BlindScoringRequest` (`exit 1`).
    InvalidTalkCommitment(String),
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
    /// On-disk checkpoint `.save.json` or `.seal` differs (or is missing) at the post-batch check (`exit 2`).
    CheckpointChangedDuringBatch(String),
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
            Self::StateIdFilenameMismatch(_) => "STATE_ID_FILENAME_MISMATCH",
            Self::InvalidHexDigest { .. } => "INVALID_HEX_DIGEST",
            Self::InvalidFocalSeat { .. } => "INVALID_FOCAL_SEAT",
            Self::EmptyCandidates(_) => "EMPTY_CANDIDATES",
            Self::NoInSpanCandidates(_) => "NO_IN_SPAN_CANDIDATES",
            Self::UnsortedCandidates { .. } => "UNSORTED_CANDIDATES",
            Self::DuplicateCandidateId(_) => "DUPLICATE_CANDIDATE_ID",
            Self::CandidateIdMismatch { .. } => "CANDIDATE_ID_MISMATCH",
            Self::UnknownSubmissionField(_) => "UNKNOWN_SUBMISSION_FIELD",
            Self::ConfigHashMismatch { .. } => "CONFIG_HASH_MISMATCH",
            Self::FreezeConfigMismatch(_) => "FREEZE_CONFIG_MISMATCH",
            Self::FreezeTreeMismatch(_) => "FREEZE_TREE_MISMATCH",
            Self::FreezeManifestMismatch(_) => "FREEZE_MANIFEST_MISMATCH",
            Self::MissingVerifiedCheckpoints(_) => "MISSING_VERIFIED_CHECKPOINTS",
            Self::FreezeVerifiedCheckpointsMismatch(_) => "FREEZE_VERIFIED_CHECKPOINTS_MISMATCH",
            Self::CheckpointVerify(err) => err.code(),
            Self::InvalidTalkCommitment(_) => "INVALID_TALK_COMMITMENT",
            Self::InvalidScorerConfig(_) => "INVALID_SCORER_CONFIG",
            Self::Json(_) => "INVALID_JSON",
            Self::NoBlindInputs(_) => "NO_BLIND_INPUTS",
            Self::InvalidCheckpointsLayout(_) => "INVALID_CHECKPOINTS_LAYOUT",
            Self::CheckpointNotFound(_) => "CHECKPOINT_NOT_FOUND",
            Self::CheckpointHashMismatch(_) => "CHECKPOINT_HASH_MISMATCH",
            Self::InvalidCheckpointState(_) => "INVALID_CHECKPOINT_STATE",
            Self::FocalMandateNotInDeck => "FOCAL_MANDATE_NOT_IN_DECK",
            Self::InsufficientMandatePool { .. } => "INSUFFICIENT_MANDATE_POOL",
            Self::CheckpointChangedDuringBatch(_) => "CHECKPOINT_CHANGED_DURING_BATCH",
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
            | Self::StateIdFilenameMismatch(_)
            | Self::InvalidHexDigest { .. }
            | Self::InvalidFocalSeat { .. }
            | Self::EmptyCandidates(_)
            | Self::NoInSpanCandidates(_)
            | Self::UnsortedCandidates { .. }
            | Self::DuplicateCandidateId(_)
            | Self::CandidateIdMismatch { .. }
            | Self::UnknownSubmissionField(_)
            | Self::ConfigHashMismatch { .. }
            | Self::FreezeConfigMismatch(_)
            | Self::FreezeTreeMismatch(_)
            | Self::FreezeManifestMismatch(_)
            | Self::MissingVerifiedCheckpoints(_)
            | Self::FreezeVerifiedCheckpointsMismatch(_)
            | Self::CheckpointVerify(
                CheckpointVerifyError::OuterFileHashMismatch { .. }
                | CheckpointVerifyError::UnexpectedEofOrMalformedEncoding { .. }
                | CheckpointVerifyError::PairBindingMismatch { .. }
                | CheckpointVerifyError::TipStateDigestMismatch { .. }
                | CheckpointVerifyError::NonCanonicalEncoding { .. }
                | CheckpointVerifyError::UnknownFieldRejected { .. }
                | CheckpointVerifyError::IncompatibleEngineOrSchemaVersion { .. },
            )
            | Self::InvalidTalkCommitment(_)
            | Self::InvalidScorerConfig(_)
            | Self::Json(_)
            | Self::NoBlindInputs(_)
            | Self::InvalidCheckpointsLayout(_)
            | Self::CheckpointNotFound(_)
            | Self::CheckpointHashMismatch(_)
            | Self::InvalidCheckpointState(_) => EXIT_USER_ERROR,
            Self::CheckpointVerify(
                CheckpointVerifyError::ReplayDigestMismatch { .. }
                | CheckpointVerifyError::ChainHashMismatch { .. }
                | CheckpointVerifyError::ProbeStepDivergence { .. }
                | CheckpointVerifyError::InMemoryBaseStateCorrupted { .. },
            )
            | Self::FocalMandateNotInDeck
            | Self::InsufficientMandatePool { .. }
            | Self::CheckpointChangedDuringBatch(_)
            | Self::Engine(_)
            | Self::Io(_) => EXIT_RUNTIME_ERROR,
        }
    }

    /// Human-readable remediation hint matching [`ToStructuredError`].
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Usage(_) => "Run `league-scorer --help` and verify all required flags.",
            Self::UnsupportedSchemaVersion(_) => {
                "Generate blind scoring requests with schema_version = 1."
            }
            Self::InvalidStateId(_) => {
                "Use unique case-insensitive ASCII state_ids matching `<source_game_id>-day-<DD>-seat-<idx>`."
            }
            Self::StateIdFilenameMismatch(_) => {
                "Rename each blind input JSON file so its stem matches `request.state_id`."
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
            Self::UnknownSubmissionField(_) => {
                "Serialize each candidate submission using strict EvalActSubmission schema fields only."
            }
            Self::ConfigHashMismatch { .. } => {
                "Pass the matching ScorerConfig JSON or rebuild requests against the frozen config."
            }
            Self::FreezeConfigMismatch(_) => {
                "Pass the ScorerConfig frozen inside scorer_freeze.json."
            }
            Self::FreezeTreeMismatch(_) => {
                "Run the exact league-scorer revision whose tree digest is recorded in scorer_freeze.json."
            }
            Self::FreezeManifestMismatch(_) => {
                "Pass the exact blind_inputs directory whose manifest digest is recorded in scorer_freeze.json."
            }
            Self::MissingVerifiedCheckpoints(_) => {
                "Run `league-eval verify-checkpoints` and pass `--verified-checkpoints <file>` (recorded as `verified_checkpoints_sha256` in scorer_freeze.json)."
            }
            Self::FreezeVerifiedCheckpointsMismatch(_) => {
                "Pass the exact verified_checkpoints.json whose SHA-256 is recorded in scorer_freeze.json."
            }
            Self::CheckpointVerify(err) => err.hint(),
            Self::InvalidTalkCommitment(_) => {
                "Ensure commitment seats are distinct in-range peer seats matching offered_action."
            }
            Self::InvalidScorerConfig(_) => {
                "Verify scenarios_m is even >= 2, posture settings are valid, and all config fields are well-formed."
            }
            Self::Json(_) => "Ensure request and config JSON match the strict scorer schemas.",
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
            Self::CheckpointChangedDuringBatch(_) => {
                "Keep --checkpoints immutable while scoring runs and re-score the entire batch from verified checkpoint files."
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
            | Self::StateIdFilenameMismatch(msg)
            | Self::EmptyCandidates(msg)
            | Self::NoInSpanCandidates(msg)
            | Self::DuplicateCandidateId(msg)
            | Self::UnknownSubmissionField(msg)
            | Self::FreezeConfigMismatch(msg)
            | Self::FreezeTreeMismatch(msg)
            | Self::FreezeManifestMismatch(msg)
            | Self::MissingVerifiedCheckpoints(msg)
            | Self::FreezeVerifiedCheckpointsMismatch(msg)
            | Self::InvalidTalkCommitment(msg)
            | Self::InvalidScorerConfig(msg)
            | Self::Json(msg)
            | Self::NoBlindInputs(msg)
            | Self::InvalidCheckpointsLayout(msg)
            | Self::CheckpointNotFound(msg)
            | Self::CheckpointHashMismatch(msg)
            | Self::InvalidCheckpointState(msg)
            | Self::CheckpointChangedDuringBatch(msg)
            | Self::Engine(msg)
            | Self::Io(msg) => write!(f, "{code}: {msg}"),
            Self::CheckpointVerify(err) => fmt::Display::fmt(err, f),
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

impl From<CheckpointVerifyError> for ScorerError {
    fn from(value: CheckpointVerifyError) -> Self {
        Self::CheckpointVerify(value)
    }
}

impl ToStructuredError for ScorerError {
    fn to_structured_error(&self) -> StructuredError {
        StructuredError::new(self.code(), self.to_string(), self.hint(), false)
    }
}

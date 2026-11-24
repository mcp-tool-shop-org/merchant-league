//! Causal, canonical, and pair-bound verification for Evaluation Mode pre-act checkpoints.
//!
//! Implements Gate 2 preflight verification (Turn-0 deterministic replay, cumulative turn chain
//! digest `C_k`, tip-state equivalence, and a 1-turn `ClubBot` probe step) and Gate 3 worker load
//! checks (outer file SHA-256 verification, strict duplicate/unknown-key JSON decoding, canonical
//! byte round-trip checks, pair binding, and canonical tip-state digest verification).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::bot::ClubBot;
use crate::commit::{draw_shocks_for_turn, verify_eval_vrf_beacon};
use crate::engine::TurnEngine;
use crate::error::{StructuredError, ToStructuredError};
use crate::types::{EngineError, HARD_TURN_CAP, Seat, TableMode, Turn};

use super::{
    EVAL_SAVE_FORMAT_VERSION, EVAL_SEAL_FORMAT_VERSION, EvalActSubmission, EvalHost,
    EvalSaveSnapshot, EvalSealPayload, compute_pre_act_pair_sha256_hex, is_lower_ascii_hex_len,
    validate_eval_seal_bytes,
};

/// Domain separator for `EvalSaveSnapshot` v2 content binding inside `EvalSealPayload::save_sha256`.
pub const EVAL_SAVE_V2_HASH_DOMAIN: &[u8] = b"league-core/eval-save/v2|";

/// Domain separator for Turn-0 initialization of the cumulative turn-chain digest `C_0`.
pub const EVAL_TURN_CHAIN_INIT_DOMAIN: &[u8] = b"league-core/eval-turn-chain-init/v1|";

/// Domain separator per completed turn `k` in cumulative turn-chain digest `C_k`.
pub const EVAL_TURN_CHAIN_STEP_DOMAIN: &[u8] = b"league-core/eval-turn-chain/v1|";

/// Computes lowercase 64-hex `SHA-256` of `bytes`.
#[must_use]
pub fn sha256_bytes_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    hex::encode(digest)
}

/// Computes lowercase 64-hex `SHA-256("league-core/eval-save/v2|" || save_bytes)`.
#[must_use]
pub fn compute_eval_save_v2_sha256_hex(save_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(EVAL_SAVE_V2_HASH_DOMAIN);
    hasher.update(save_bytes);
    let digest: [u8; 32] = hasher.finalize().into();
    hex::encode(digest)
}

/// Returns `true` iff `raw` is a 40-character lowercase ASCII hexadecimal git commit SHA.
#[must_use]
pub fn is_valid_git_commit_sha40(raw: &str) -> bool {
    is_lower_ascii_hex_len(raw, 40)
}

/// Typed causal and encoding verification failures raised across Gate 1, Gate 2, and Gate 3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointVerifyError {
    /// On-disk checkpoint bytes disagree with manifest or request SHA-256 digests (`M01`).
    OuterFileHashMismatch {
        /// Logical file role (`save`, `seal`, or `pair`).
        file: String,
        /// Expected 64-hex SHA-256 digest.
        expected: String,
        /// Actual observed 64-hex SHA-256 digest.
        actual: String,
    },
    /// JSON syntax error, premature EOF, non-UTF-8 input, or trailing garbage token (`M02`).
    UnexpectedEofOrMalformedEncoding {
        /// Logical file role (`save` or `seal`).
        file: String,
        /// 1-based line number from parser, when available.
        line: usize,
        /// 1-based column number from parser, when available.
        column: usize,
        /// Parser failure description.
        detail: String,
    },
    /// `.save.json` and `.seal` fail `game_id`, `layout_seed`, `turn`, or `save_sha256` binding (`M03`).
    PairBindingMismatch {
        /// Concise cause description.
        reason: String,
    },
    /// Loaded tip state disagrees with `live_state_digests`, `.save.json`, manifest, or replay tip (`M04`).
    TipStateDigestMismatch {
        /// Tip turn number on the checkpoint.
        turn: u8,
        /// Expected digest or count string.
        expected: String,
        /// Actual observed digest or count string.
        actual: String,
        /// Contextual detail.
        detail: String,
    },
    /// Deterministic Turn-0 replay diverges from `logged_turns` / `live_state_digests` at `turn` (`M05`, `M06`).
    ReplayDigestMismatch {
        /// 1-based completed historical turn at which replay diverged.
        turn: u8,
        /// Expected digest from checkpoint sidecar or replay specification.
        expected: String,
        /// Observed replayed digest or error detail.
        actual: String,
    },
    /// Cumulative turn-chain digest `C_k` diverges from expected chain digest at `turn` (`M06`).
    ChainHashMismatch {
        /// Turn at which cumulative chain diverged.
        turn: u8,
        /// Expected 64-hex chain digest.
        expected: String,
        /// Computed 64-hex chain digest.
        actual: String,
    },
    /// Deterministic 1-turn `ClubBot` probe step diverges between replayed and loaded hosts.
    ProbeStepDivergence {
        /// Pre-act tip turn on which the probe ran.
        turn: u8,
        /// Post-probe canonical state digest on the Turn-0 replayed host.
        replayed_digest: String,
        /// Post-probe canonical state digest on the loaded checkpoint host.
        loaded_digest: String,
    },
    /// Non-canonical JSON encoding: key reordering, extra whitespace, duplicate keys, or omitted-default keys (`M07`).
    NonCanonicalEncoding {
        /// Logical file role (`save` or `seal`).
        file: String,
        /// Exact non-canonical defect detail.
        detail: String,
    },
    /// Unrecognized JSON key at top level or nested inside state/seal (`M08`).
    UnknownFieldRejected {
        /// Logical file role (`save` or `seal`).
        file: String,
        /// Dotted JSON path of the unrecognized field.
        field_path: String,
    },
    /// Incompatible format version or missing/invalid/mismatched engine commit or build id (`M09`).
    IncompatibleEngineOrSchemaVersion {
        /// Field or header that failed compatibility validation.
        field: String,
        /// Value found in checkpoint or runtime environment.
        found: String,
        /// Required version or commit specification.
        expected: String,
    },
    /// Gate 3 post-batch canary detected in-memory mutation of the shared base `EvalHost` (`M10`).
    InMemoryBaseStateCorrupted {
        /// Pre-batch base canonical digest.
        expected: String,
        /// Post-batch base canonical digest.
        actual: String,
    },
}

impl CheckpointVerifyError {
    /// Stable uppercase machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::OuterFileHashMismatch { .. } => "OUTER_FILE_HASH_MISMATCH",
            Self::UnexpectedEofOrMalformedEncoding { .. } => "UNEXPECTED_EOF_OR_MALFORMED_ENCODING",
            Self::PairBindingMismatch { .. } => "PAIR_BINDING_MISMATCH",
            Self::TipStateDigestMismatch { .. } => "TIP_STATE_DIGEST_MISMATCH",
            Self::ReplayDigestMismatch { .. } => "REPLAY_DIGEST_MISMATCH",
            Self::ChainHashMismatch { .. } => "CHAIN_HASH_MISMATCH",
            Self::ProbeStepDivergence { .. } => "PROBE_STEP_DIVERGENCE",
            Self::NonCanonicalEncoding { .. } => "NON_CANONICAL_ENCODING",
            Self::UnknownFieldRejected { .. } => "UNKNOWN_FIELD_REJECTED",
            Self::IncompatibleEngineOrSchemaVersion { .. } => {
                "INCOMPATIBLE_ENGINE_OR_SCHEMA_VERSION"
            }
            Self::InMemoryBaseStateCorrupted { .. } => "IN_MEMORY_BASE_STATE_CORRUPTED",
        }
    }

    /// Operator remediation hint matching [`ToStructuredError`].
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::OuterFileHashMismatch { .. } => {
                "Re-export uncorrupted checkpoint files matching expected SHA-256 digests."
            }
            Self::UnexpectedEofOrMalformedEncoding { .. } => {
                "Re-write complete, valid UTF-8 JSON checkpoint files without truncation."
            }
            Self::PairBindingMismatch { .. } => {
                "Pair each .save.json strictly with the .seal generated alongside it for the same game_id and turn."
            }
            Self::TipStateDigestMismatch { .. } => {
                "Re-verify checkpoint provenance and ensure tip state matches live_state_digests, save snapshot, and manifest."
            }
            Self::ReplayDigestMismatch { .. } => {
                "Re-generate the checkpoint from a clean matching engine build; historical replay diverged."
            }
            Self::ChainHashMismatch { .. } => {
                "Ensure logged_turns match the cumulative turn-chain digest sealed at preflight."
            }
            Self::ProbeStepDivergence { .. } => {
                "Investigate state determinism mismatch between replayed and loaded pre-act hosts."
            }
            Self::NonCanonicalEncoding { .. } => {
                "Emit checkpoint files strictly via EvalHost canonical serialization without reformatting or duplicate keys."
            }
            Self::UnknownFieldRejected { .. } => {
                "Strip unrecognized JSON fields or align checkpoint format version with this build."
            }
            Self::IncompatibleEngineOrSchemaVersion { .. } => {
                "Rebuild cleanly inside git with a 40-hex commit SHA and supply format_version = 2 checkpoints matching this engine."
            }
            Self::InMemoryBaseStateCorrupted { .. } => {
                "Abort scoring batch immediately; the in-memory base EvalHost mutated during rollout execution."
            }
        }
    }

    pub(super) fn into_engine_error(self) -> EngineError {
        match self {
            Self::IncompatibleEngineOrSchemaVersion {
                ref field,
                ref found,
                ref expected,
            } if (field == "save.format_version" || field == "seal.format_version") => {
                if let (Ok(found_u32), Ok(expected_u32)) =
                    (found.parse::<u32>(), expected.parse::<u32>())
                {
                    return EngineError::UnsupportedReplayVersion {
                        found: found_u32,
                        newest_known: expected_u32,
                    };
                }
                EngineError::InvalidConfig(format!("{}: {self}", self.code()))
            }
            Self::UnexpectedEofOrMalformedEncoding { .. }
            | Self::PairBindingMismatch { .. }
            | Self::TipStateDigestMismatch { .. }
            | Self::NonCanonicalEncoding { .. }
            | Self::UnknownFieldRejected { .. }
            | Self::OuterFileHashMismatch { .. }
            | Self::ReplayDigestMismatch { .. }
            | Self::ChainHashMismatch { .. }
            | Self::ProbeStepDivergence { .. }
            | Self::IncompatibleEngineOrSchemaVersion { .. }
            | Self::InMemoryBaseStateCorrupted { .. } => {
                EngineError::InvalidConfig(format!("{}: {self}", self.code()))
            }
        }
    }
}

impl fmt::Display for CheckpointVerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.code();
        match self {
            Self::OuterFileHashMismatch {
                file,
                expected,
                actual,
            } => write!(
                f,
                "{code}: `{file}` SHA-256 mismatch (expected `{expected}`, got `{actual}`)"
            ),
            Self::UnexpectedEofOrMalformedEncoding {
                file,
                line,
                column,
                detail,
            } => write!(
                f,
                "{code}: malformed `{file}` JSON at line {line}, column {column}: {detail}"
            ),
            Self::PairBindingMismatch { reason } => write!(f, "{code}: {reason}"),
            Self::TipStateDigestMismatch {
                turn,
                expected,
                actual,
                detail,
            } => write!(
                f,
                "{code}: tip turn {turn} state mismatch ({detail}; expected `{expected}`, got `{actual}`)"
            ),
            Self::ReplayDigestMismatch {
                turn,
                expected,
                actual,
            } => write!(
                f,
                "{code}: Turn-0 replay diverged at turn {turn} (expected `{expected}`, got `{actual}`)"
            ),
            Self::ChainHashMismatch {
                turn,
                expected,
                actual,
            } => write!(
                f,
                "{code}: cumulative chain digest C_{turn} mismatch (expected `{expected}`, got `{actual}`)"
            ),
            Self::ProbeStepDivergence {
                turn,
                replayed_digest,
                loaded_digest,
            } => write!(
                f,
                "{code}: 1-turn ClubBot probe diverged at turn {turn} (replayed `{replayed_digest}`, loaded `{loaded_digest}`)"
            ),
            Self::NonCanonicalEncoding { file, detail } => {
                write!(f, "{code}: non-canonical `{file}` encoding: {detail}")
            }
            Self::UnknownFieldRejected { file, field_path } => {
                write!(f, "{code}: unknown field `{field_path}` in `{file}`")
            }
            Self::IncompatibleEngineOrSchemaVersion {
                field,
                found,
                expected,
            } => write!(
                f,
                "{code}: `{field}` found `{found}`, expected `{expected}`"
            ),
            Self::InMemoryBaseStateCorrupted { expected, actual } => write!(
                f,
                "{code}: in-memory base EvalHost mutated (expected `{expected}`, got `{actual}`)"
            ),
        }
    }
}

impl std::error::Error for CheckpointVerifyError {}

fn incompatible_err(
    field: &str,
    found: impl Into<String>,
    expected: &str,
) -> CheckpointVerifyError {
    CheckpointVerifyError::IncompatibleEngineOrSchemaVersion {
        field: field.to_owned(),
        found: found.into(),
        expected: expected.to_owned(),
    }
}

fn replay_err(turn: u8, expected: &str, actual: impl ToString) -> CheckpointVerifyError {
    CheckpointVerifyError::ReplayDigestMismatch {
        turn,
        expected: expected.to_owned(),
        actual: actual.to_string(),
    }
}

fn tip_err(turn: u8, expected: &str, actual: impl ToString, detail: &str) -> CheckpointVerifyError {
    CheckpointVerifyError::TipStateDigestMismatch {
        turn,
        expected: expected.to_owned(),
        actual: actual.to_string(),
        detail: detail.to_owned(),
    }
}

fn non_canonical_err(file: &str, detail: impl ToString) -> CheckpointVerifyError {
    CheckpointVerifyError::NonCanonicalEncoding {
        file: file.to_owned(),
        detail: detail.to_string(),
    }
}

impl ToStructuredError for CheckpointVerifyError {
    fn to_structured_error(&self) -> StructuredError {
        StructuredError::new(self.code(), self.to_string(), self.hint(), false)
    }
}

/// Secret-free Gate 2 verification summary emitted for each verified checkpoint pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCheckpointCausalSummary {
    /// Content-addressed domain-separated pair SHA-256 of `.save.json` + `.seal`.
    pub pre_act_pair_sha256: String,
    /// Lowercase 64-hex SHA-256 of `.save.json` bytes.
    pub pre_act_save_sha256: String,
    /// Lowercase 64-hex SHA-256 of `.seal` bytes.
    pub pre_act_seal_sha256: String,
    /// Recorded 40-hex commit SHA provenance copied from `EvalSealPayload::engine_commit`.
    pub engine_commit: String,
    /// Verified pre-act tip turn (`1..=32`).
    pub turn: u8,
    /// Table seat count (`4` for Evaluation Mode).
    pub seat_count: u8,
    /// Lowercase 64-hex canonical `TurnEngine::state_digest()` at tip `turn`.
    pub expected_state_digest: String,
    /// Lowercase 64-hex cumulative chain digest `C_{turn - 1}`.
    pub turn_chain_digest: String,
    /// Lowercase 64-hex canonical `TurnEngine::state_digest()` after 1-turn `ClubBot` probe sweep.
    pub post_probe_state_digest: String,
}

const DUPLICATE_KEY_PREFIX: &str = "__DUPLICATE_JSON_KEY__:";

struct StrictJsonValue(Value);

struct StrictJsonValueVisitor;

impl<'de> Visitor<'de> for StrictJsonValueVisitor {
    type Value = StrictJsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any valid JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(Value::Bool(v)))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(Value::Number(v.into())))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(Value::Number(v.into())))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .map(StrictJsonValue)
            .ok_or_else(|| serde::de::Error::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(Value::String(v.to_owned())))
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(Value::String(v)))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(Value::Null))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        Deserialize::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(elem) = seq.next_element::<StrictJsonValue>()? {
            out.push(elem.0);
        }
        Ok(StrictJsonValue(Value::Array(out)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut out = Map::new();
        let mut seen = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(serde::de::Error::custom(format!(
                    "{DUPLICATE_KEY_PREFIX}{key}"
                )));
            }
            let val = map.next_value::<StrictJsonValue>()?;
            out.insert(key, val.0);
        }
        Ok(StrictJsonValue(Value::Object(out)))
    }
}

impl<'de> Deserialize<'de> for StrictJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictJsonValueVisitor)
    }
}

/// Parses raw JSON bytes while rejecting syntax/EOF errors and duplicate object keys at any depth.
pub fn parse_strict_json_value(
    file_label: &str,
    bytes: &[u8],
) -> Result<Value, CheckpointVerifyError> {
    if bytes.is_empty() {
        return Err(CheckpointVerifyError::UnexpectedEofOrMalformedEncoding {
            file: file_label.to_owned(),
            line: 1,
            column: 1,
            detail: "empty JSON input".to_owned(),
        });
    }
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let value = StrictJsonValue::deserialize(&mut de).map_err(|err| {
        let msg = err.to_string();
        if let Some(rest) = msg.split_once(DUPLICATE_KEY_PREFIX).map(|(_, k)| k) {
            CheckpointVerifyError::NonCanonicalEncoding {
                file: file_label.to_owned(),
                detail: format!("duplicate JSON object key `{rest}`"),
            }
        } else {
            CheckpointVerifyError::UnexpectedEofOrMalformedEncoding {
                file: file_label.to_owned(),
                line: err.line(),
                column: err.column(),
                detail: msg,
            }
        }
    })?;
    de.end().map_err(
        |err| CheckpointVerifyError::UnexpectedEofOrMalformedEncoding {
            file: file_label.to_owned(),
            line: err.line(),
            column: err.column(),
            detail: format!("trailing data after root JSON value: {err}"),
        },
    )?;
    Ok(value.0)
}

fn check_version_header_raw(
    field_name: &str,
    raw: &Value,
    expected_version: u32,
) -> Result<(), CheckpointVerifyError> {
    if let Some(found) = raw.get("format_version") {
        let found_str = match found.as_u64() {
            Some(v) => {
                if v == u64::from(expected_version) {
                    return Ok(());
                }
                v.to_string()
            }
            None => found.to_string(),
        };
        return Err(CheckpointVerifyError::IncompatibleEngineOrSchemaVersion {
            field: field_name.to_owned(),
            found: found_str,
            expected: expected_version.to_string(),
        });
    }
    Err(CheckpointVerifyError::IncompatibleEngineOrSchemaVersion {
        field: field_name.to_owned(),
        found: "missing".to_owned(),
        expected: expected_version.to_string(),
    })
}

fn is_known_skip_serializing_field(
    file_label: &str,
    parent_path: &str,
    key: &str,
    raw_val: &Value,
) -> bool {
    if file_label != "seal" {
        return false;
    }
    if parent_path.is_empty()
        && matches!(
            key,
            "pre_act_checkpoint_sha256" | "save_sha256" | "engine_commit"
        )
        && raw_val.is_null()
    {
        return true;
    }
    if parent_path == "engine"
        && key == "public_receipts"
        && raw_val.as_array().is_some_and(Vec::is_empty)
    {
        return true;
    }
    if parent_path.starts_with("logged_turns[") && key == "cancel_lock_at" && raw_val.is_null() {
        return true;
    }
    if parent_path.starts_with("recorded_act_submissions.")
        && key == "pass_b_actions"
        && raw_val.as_array().is_some_and(Vec::is_empty)
    {
        return true;
    }
    false
}

/// Checks recursively that every key in `raw` exists in `canonical`, distinguishing explicitly emitted
/// `skip_serializing_if` empty/null defaults (`NonCanonicalEncoding`) from unrecognized keys (`UnknownFieldRejected`).
pub(super) fn check_unknown_and_skipped_fields(
    file_label: &str,
    path: &str,
    raw: &Value,
    canonical: &Value,
) -> Result<(), CheckpointVerifyError> {
    match (raw, canonical) {
        (Value::Object(raw_map), Value::Object(can_map)) => {
            for (k, raw_v) in raw_map {
                let child_path = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                match can_map.get(k) {
                    Some(can_v) => {
                        check_unknown_and_skipped_fields(file_label, &child_path, raw_v, can_v)?;
                    }
                    None => {
                        if is_known_skip_serializing_field(file_label, path, k, raw_v) {
                            return Err(CheckpointVerifyError::NonCanonicalEncoding {
                                file: file_label.to_owned(),
                                detail: format!(
                                    "default-skipped optional field emitted explicitly at `{child_path}`"
                                ),
                            });
                        }
                        return Err(CheckpointVerifyError::UnknownFieldRejected {
                            file: file_label.to_owned(),
                            field_path: child_path,
                        });
                    }
                }
            }
        }
        (Value::Array(raw_arr), Value::Array(can_arr)) => {
            for (idx, (raw_item, can_item)) in raw_arr.iter().zip(can_arr.iter()).enumerate() {
                let child_path = format!("{path}[{idx}]");
                check_unknown_and_skipped_fields(file_label, &child_path, raw_item, can_item)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn parse_strict_typed_save(
    save_bytes: &[u8],
) -> Result<(EvalSaveSnapshot, Value), CheckpointVerifyError> {
    let raw_save = parse_strict_json_value("save", save_bytes)?;
    check_version_header_raw("save.format_version", &raw_save, EVAL_SAVE_FORMAT_VERSION)?;
    let save: EvalSaveSnapshot = serde_json::from_value(raw_save.clone()).map_err(|err| {
        CheckpointVerifyError::UnexpectedEofOrMalformedEncoding {
            file: "save".to_owned(),
            line: err.line(),
            column: err.column(),
            detail: format!("Eval save schema mismatch: {err}"),
        }
    })?;
    let canonical_save =
        serde_json::to_value(&save).map_err(|err| CheckpointVerifyError::NonCanonicalEncoding {
            file: "save".to_owned(),
            detail: format!("failed to canonicalize parsed save: {err}"),
        })?;
    check_unknown_and_skipped_fields("save", "", &raw_save, &canonical_save)?;
    Ok((save, raw_save))
}

pub(super) fn parse_strict_typed_seal_raw(
    seal_bytes: &[u8],
) -> Result<(EvalSealPayload, Value), CheckpointVerifyError> {
    let raw_seal = parse_strict_json_value("seal", seal_bytes)?;
    check_version_header_raw("seal.format_version", &raw_seal, EVAL_SEAL_FORMAT_VERSION)?;
    match raw_seal.get("save_format_version") {
        Some(v) if v.as_u64() == Some(u64::from(EVAL_SAVE_FORMAT_VERSION)) => {}
        Some(v) => {
            return Err(incompatible_err(
                "seal.save_format_version",
                v.to_string(),
                &EVAL_SAVE_FORMAT_VERSION.to_string(),
            ));
        }
        None => {
            return Err(incompatible_err(
                "seal.save_format_version",
                "missing",
                &EVAL_SAVE_FORMAT_VERSION.to_string(),
            ));
        }
    }
    match raw_seal.get("engine_commit") {
        Some(Value::String(sha)) if is_valid_git_commit_sha40(sha) => {}
        Some(other) => {
            return Err(incompatible_err(
                "seal.engine_commit",
                other.to_string(),
                "40 lowercase hex git commit sha",
            ));
        }
        None => {
            return Err(incompatible_err(
                "seal.engine_commit",
                "missing",
                "40 lowercase hex git commit sha",
            ));
        }
    }

    let seal: EvalSealPayload = serde_json::from_value(raw_seal.clone()).map_err(|err| {
        CheckpointVerifyError::UnexpectedEofOrMalformedEncoding {
            file: "seal".to_owned(),
            line: err.line(),
            column: err.column(),
            detail: format!("Eval seal schema mismatch: {err}"),
        }
    })?;
    let canonical_seal =
        serde_json::to_value(&seal).map_err(|err| CheckpointVerifyError::NonCanonicalEncoding {
            file: "seal".to_owned(),
            detail: format!("failed to canonicalize parsed seal: {err}"),
        })?;
    check_unknown_and_skipped_fields("seal", "", &raw_seal, &canonical_seal)?;
    Ok((seal, raw_seal))
}

/// Executes Gate 1 strict round-trip decoding and canonical byte equality self-check on a
/// freshly serialized `.save.json` / `.seal` pair before writing it to disk.
pub(super) fn verify_written_pair_round_trip(
    save_bytes: &[u8],
    seal_bytes: &[u8],
) -> Result<(), CheckpointVerifyError> {
    let (save, _) = parse_strict_typed_save(save_bytes)?;
    let (seal, _) = parse_strict_typed_seal_raw(seal_bytes)?;
    let expected_save_sha = compute_eval_save_v2_sha256_hex(save_bytes);
    if save.game_id != seal.game_id {
        return Err(CheckpointVerifyError::PairBindingMismatch {
            reason: format!(
                "written save.game_id `{}` != seal.game_id `{}`",
                save.game_id, seal.game_id
            ),
        });
    }
    if seal.save_sha256.as_deref() != Some(expected_save_sha.as_str()) {
        return Err(CheckpointVerifyError::PairBindingMismatch {
            reason: "written seal.save_sha256 disagrees with domain SHA-256 of written save"
                .to_owned(),
        });
    }
    let save_rt = serde_json::to_vec_pretty(&save)
        .map_err(|err| non_canonical_err("save", format!("re-serialize written save: {err}")))?;
    if save_rt != save_bytes {
        return Err(non_canonical_err(
            "save",
            "written save bytes failed canonical round-trip check",
        ));
    }
    let seal_rt = serde_json::to_vec(&seal)
        .map_err(|err| non_canonical_err("seal", format!("re-serialize written seal: {err}")))?;
    if seal_rt != seal_bytes {
        return Err(non_canonical_err(
            "seal",
            "written seal bytes failed canonical round-trip check",
        ));
    }
    Ok(())
}

/// Strictly decodes and verifies a `.save.json` / `.seal` pre-act checkpoint pair:
/// duplicate-key free UTF-8 JSON, `format_version == 2`, valid 40-hex `engine_commit`,
/// zero unknown or emitted-default fields, canonical byte round-trip equality, pre-act
/// state invariants, length parity, tip digest equality, snapshot parity, `game_id` parity,
/// and domain-separated `save_sha256` binding.
pub fn decode_strict_checkpoint_pair(
    save_bytes: &[u8],
    seal_bytes: &[u8],
) -> Result<EvalHost, CheckpointVerifyError> {
    let (save, _) = parse_strict_typed_save(save_bytes)?;
    let (seal_header, _) = parse_strict_typed_seal_raw(seal_bytes)?;

    if !is_lower_ascii_hex_len(&save.game_id, 32) {
        return Err(CheckpointVerifyError::PairBindingMismatch {
            reason: format!(
                "save.game_id must be 32 lowercase hex characters, got `{}`",
                save.game_id
            ),
        });
    }
    if save.game_id != seal_header.game_id {
        return Err(CheckpointVerifyError::PairBindingMismatch {
            reason: format!(
                "save.game_id `{}` != seal.game_id `{}`",
                save.game_id, seal_header.game_id
            ),
        });
    }
    if seal_header.turn != seal_header.engine.state().turn {
        return Err(tip_err(
            seal_header.engine.state().turn,
            &seal_header.engine.state().turn.to_string(),
            seal_header.turn,
            "seal header turn != seal.engine.state().turn",
        ));
    }

    let seal_turn = seal_header.engine.state().turn;
    if seal_turn == 0 || seal_turn > HARD_TURN_CAP {
        return Err(tip_err(
            seal_turn,
            &format!("1..={HARD_TURN_CAP}"),
            seal_turn,
            "tip turn out of valid range",
        ));
    }
    if save.turn != seal_turn {
        return Err(tip_err(
            seal_turn,
            &seal_turn.to_string(),
            save.turn,
            "save.turn != seal.engine.state().turn",
        ));
    }
    let expected_history_len = usize::from(seal_turn - 1);
    if seal_header.logged_turns.len() != expected_history_len {
        return Err(tip_err(
            seal_turn,
            &expected_history_len.to_string(),
            seal_header.logged_turns.len(),
            "seal.logged_turns.len() != turn - 1",
        ));
    }
    if seal_header.live_state_digests.len() != expected_history_len {
        return Err(tip_err(
            seal_turn,
            &expected_history_len.to_string(),
            seal_header.live_state_digests.len(),
            "seal.live_state_digests.len() != turn - 1",
        ));
    }

    let expected_canonical_seal = serde_json::to_vec(&seal_header).map_err(|err| {
        non_canonical_err("seal", format!("re-serialize seal canonically: {err}"))
    })?;
    if expected_canonical_seal != seal_bytes {
        return Err(non_canonical_err(
            "seal",
            "seal bytes do not match compact canonical serde_json output",
        ));
    }

    let expected_canonical_save = serde_json::to_vec_pretty(&save).map_err(|err| {
        non_canonical_err("save", format!("re-serialize save canonically: {err}"))
    })?;
    if expected_canonical_save != save_bytes {
        return Err(non_canonical_err(
            "save",
            "save bytes do not match pretty canonical serde_json output",
        ));
    }

    let seal = validate_eval_seal_bytes(seal_bytes).map_err(|err| {
        CheckpointVerifyError::PairBindingMismatch {
            reason: err.to_string(),
        }
    })?;
    if seal.pre_act_checkpoint_sha256.is_some() {
        return Err(CheckpointVerifyError::PairBindingMismatch {
            reason: "pre-act seal must not carry act-recorded pre_act_checkpoint_sha256".to_owned(),
        });
    }
    let expected_save_sha = compute_eval_save_v2_sha256_hex(save_bytes);
    if seal.save_sha256.as_deref() != Some(expected_save_sha.as_str()) {
        return Err(CheckpointVerifyError::PairBindingMismatch {
            reason: format!(
                "seal.save_sha256 {:?} != domain SHA-256 `{expected_save_sha}`",
                seal.save_sha256
            ),
        });
    }

    let expected_layout_seed =
        super::replay_layout_seed_for_raw(seal.layout_seed).map_err(|err| {
            CheckpointVerifyError::PairBindingMismatch {
                reason: err.to_string(),
            }
        })?;
    if save.layout_seed != expected_layout_seed {
        return Err(CheckpointVerifyError::PairBindingMismatch {
            reason: "save.layout_seed does not match seal.layout_seed".to_owned(),
        });
    }

    let host = EvalHost {
        game_id: seal.game_id,
        layout_seed: seal.layout_seed,
        bot_tie_seed: seal.bot_tie_seed,
        vrf_proofs: seal.vrf_proofs,
        engine: seal.engine,
        sealed_mandate_deal: seal.sealed_mandate_deal,
        mandate_commits: seal.mandate_commits,
        mandate_reveals: seal.mandate_reveals,
        logged_turns: seal.logged_turns,
        live_state_digests: seal.live_state_digests,
        current_market_actions: seal.current_market_actions,
        current_cancel_lock_at: seal.current_cancel_lock_at,
        current_commitments: seal.current_commitments,
        current_reveals: seal.current_reveals,
        turn_salts: seal.turn_salts,
        recorded_act_submissions: seal.recorded_act_submissions,
        active_pre_act_checkpoint_sha256: Some(compute_pre_act_pair_sha256_hex(
            save_bytes, seal_bytes,
        )),
        engine_commit: seal.engine_commit,
    };

    host.ensure_pre_act_state("decode_strict_checkpoint_pair")
        .map_err(|err| CheckpointVerifyError::PairBindingMismatch {
            reason: err.to_string(),
        })?;

    let loaded_digest = host
        .canonical_state_digest_hex()
        .map_err(|err| tip_err(seal_turn, "", err, "failed to hash loaded engine state"))?;
    if seal_turn > 1 {
        let recorded_tip = hex::encode(host.live_state_digests[usize::from(seal_turn - 2)]);
        if loaded_digest != recorded_tip {
            return Err(tip_err(
                seal_turn,
                &recorded_tip,
                loaded_digest,
                "loaded engine state_digest != live_state_digests[turn - 2]",
            ));
        }
    }

    let computed_save = host
        .public_save_snapshot()
        .map_err(|err| tip_err(seal_turn, "", err, "failed to project public_save_snapshot"))?;
    if computed_save != save {
        return Err(tip_err(
            seal_turn,
            &loaded_digest,
            sha256_bytes_hex(save_bytes),
            "save snapshot diverges from loaded seal engine state",
        ));
    }

    Ok(host)
}

fn sha256_with_domain(domain: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(payload);
    hasher.finalize().into()
}

fn replay_turn_chain_step(
    prev_chain: [u8; 32],
    turn_json: &[u8],
    post_state_digest: &[u8; 32],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(EVAL_TURN_CHAIN_STEP_DOMAIN);
    hasher.update(prev_chain);
    hasher.update((turn_json.len() as u64).to_le_bytes());
    hasher.update(turn_json);
    hasher.update(post_state_digest);
    hasher.finalize().into()
}

fn execute_probe_step(host: &mut EvalHost) -> Result<(String, Vec<u8>), String> {
    let n = host.seat_count();
    let bot_tie_seed = host.bot_tie_seed();
    let mut subs = BTreeMap::new();
    for s in 0..n {
        let obs = host.observe_seat(Seat(s)).map_err(|e| e.to_string())?;
        let mandate = obs
            .own_mandate()
            .copied()
            .ok_or_else(|| format!("seat {s} missing own_mandate during probe"))?;
        let dec =
            ClubBot::new(bot_tie_seed.wrapping_add(u64::from(s))).decide_full_turn(&obs, &mandate);
        subs.insert(
            s,
            EvalActSubmission {
                market_actions: dec.pass_a_actions,
                pass_b_actions: dec.pass_b_actions,
                bundle: dec.bundle,
            },
        );
    }
    host.apply_act_submissions(&subs)
        .map_err(|e| e.to_string())?;
    let receipt = host.sweep_turn().map_err(|e| e.to_string())?;
    let post_digest = host
        .canonical_state_digest_hex()
        .map_err(|e| e.to_string())?;
    let mut engine_and_receipt = serde_json::to_vec(&host.engine).map_err(|e| e.to_string())?;
    let receipt_bytes = serde_json::to_vec(&receipt).map_err(|e| e.to_string())?;
    engine_and_receipt.extend_from_slice(&receipt_bytes);
    Ok((post_digest, engine_and_receipt))
}

fn verify_checkpoint_causal_inner(
    save_bytes: &[u8],
    seal_bytes: &[u8],
    expected_turn_chain_digest: Option<&str>,
    probe_fault_hook: Option<fn(&mut EvalHost)>,
) -> Result<(EvalHost, VerifiedCheckpointCausalSummary), CheckpointVerifyError> {
    let pre_act_save_sha256 = sha256_bytes_hex(save_bytes);
    let pre_act_seal_sha256 = sha256_bytes_hex(seal_bytes);
    let pre_act_pair_sha256 = compute_pre_act_pair_sha256_hex(save_bytes, seal_bytes);

    let loaded_host = decode_strict_checkpoint_pair(save_bytes, seal_bytes)?;
    let tip_turn = loaded_host.turn();
    let engine_commit = loaded_host.engine_commit.clone().ok_or_else(|| {
        incompatible_err(
            "seal.engine_commit",
            "missing",
            "40 lowercase hex git commit sha",
        )
    })?;
    if !is_valid_git_commit_sha40(&engine_commit) {
        return Err(incompatible_err(
            "seal.engine_commit",
            engine_commit,
            "40 lowercase hex git commit sha",
        ));
    }

    let spec = loaded_host.engine.state().spec.clone();
    if spec.mode != TableMode::Eval {
        return Err(CheckpointVerifyError::PairBindingMismatch {
            reason: "Gate 2 requires TableMode::Eval".to_owned(),
        });
    }
    let eval_vrf_pk =
        spec.eval_vrf_pubkey
            .ok_or_else(|| CheckpointVerifyError::PairBindingMismatch {
                reason: "missing TableSpec::eval_vrf_pubkey".to_owned(),
            })?;
    let mut replayed_engine =
        TurnEngine::new(spec.clone()).map_err(|err| replay_err(1, "TurnEngine::new", err))?;
    for (s_idx, commit_opt) in loaded_host.mandate_commits.iter().enumerate() {
        if let Some(commit) = commit_opt {
            replayed_engine
                .submit_mandate_commitment(Seat(s_idx as u8), *commit)
                .map_err(|err| replay_err(1, "submit_mandate_commitment", err))?;
        }
    }

    let init_digest = replayed_engine
        .state_digest()
        .map_err(|err| replay_err(1, "initial state_digest", err))?;
    let mut chain_digest = sha256_with_domain(EVAL_TURN_CHAIN_INIT_DOMAIN, &init_digest);

    for (idx, logged) in loaded_host.logged_turns.iter().enumerate() {
        let k = (idx + 1) as u8;
        let recorded_hex = hex::encode(loaded_host.live_state_digests[idx]);
        if logged.turn != k
            || logged.drand_round.is_some()
            || logged.drand_signature.is_some()
            || logged.eval_vrf_proof.as_deref() != Some(loaded_host.vrf_proofs[idx].as_slice())
        {
            return Err(replay_err(
                k,
                &recorded_hex,
                format!("invalid logged turn header or VRF proof at turn {k}"),
            ));
        }
        let vrf_proof = &loaded_host.vrf_proofs[idx];
        verify_eval_vrf_beacon(
            eval_vrf_pk,
            spec.table_id,
            Turn(k),
            vrf_proof,
            logged.turn_beacon,
        )
        .map_err(|err| replay_err(k, &recorded_hex, err))?;
        let expected_shocks =
            draw_shocks_for_turn(spec.table_id, spec.seat_count, Turn(k), logged.turn_beacon);
        if logged.drawn_shocks != expected_shocks {
            return Err(replay_err(
                k,
                &recorded_hex,
                "logged drawn_shocks diverged from HKDF shocks stream",
            ));
        }

        let action_len = logged.market_actions.len();
        let lock_at = match logged.cancel_lock_at {
            Some(pos) => {
                let pos_usize = usize::try_from(pos)
                    .map_err(|_| replay_err(k, &recorded_hex, "cancel_lock_at overflow"))?;
                if pos_usize > action_len {
                    return Err(replay_err(
                        k,
                        &recorded_hex,
                        format!("cancel_lock_at {pos} > action_len {action_len}"),
                    ));
                }
                Some(pos_usize)
            }
            None => None,
        };
        for (a_idx, action) in logged.market_actions.iter().enumerate() {
            if lock_at == Some(a_idx) {
                replayed_engine
                    .begin_cancel_lock()
                    .map_err(|err| replay_err(k, &recorded_hex, err))?;
            }
            let _ = replayed_engine.apply_market_action(action.clone());
        }
        if lock_at == Some(action_len) {
            replayed_engine
                .begin_cancel_lock()
                .map_err(|err| replay_err(k, &recorded_hex, err))?;
        }
        replayed_engine
            .advance_to_commit()
            .map_err(|err| replay_err(k, &recorded_hex, err))?;
        for (&seat_idx, &commit) in &logged.commitments {
            replayed_engine
                .submit_order_commitment(Seat(seat_idx), commit)
                .map_err(|err| replay_err(k, &recorded_hex, err))?;
        }
        replayed_engine
            .advance_to_reveal()
            .map_err(|err| replay_err(k, &recorded_hex, err))?;
        for (&seat_idx, reveal) in &logged.reveals {
            replayed_engine
                .submit_order_reveal(Seat(seat_idx), reveal.clone())
                .map_err(|err| replay_err(k, &recorded_hex, err))?;
        }
        let receipt = replayed_engine
            .sweep_turn_with_eval_vrf(vrf_proof)
            .map_err(|err| replay_err(k, &recorded_hex, err))?;
        let replayed_hex = hex::encode(receipt.post_state_digest);
        if replayed_hex != recorded_hex {
            return Err(replay_err(k, &recorded_hex, replayed_hex));
        }
        let turn_json = serde_json::to_vec(logged).map_err(|err| {
            CheckpointVerifyError::NonCanonicalEncoding {
                file: "seal".to_owned(),
                detail: format!("serialize logged_turns[{idx}] for chain hash: {err}"),
            }
        })?;
        chain_digest = replay_turn_chain_step(chain_digest, &turn_json, &receipt.post_state_digest);
    }

    let turn_chain_digest = hex::encode(chain_digest);
    if let Some(expected_chain) = expected_turn_chain_digest
        && turn_chain_digest != expected_chain
    {
        return Err(CheckpointVerifyError::ChainHashMismatch {
            turn: tip_turn.saturating_sub(1),
            expected: expected_chain.to_owned(),
            actual: turn_chain_digest,
        });
    }

    let replayed_tip_bytes = replayed_engine
        .state_digest()
        .map_err(|err| tip_err(tip_turn, "", err, "failed to digest replayed tip state"))?;
    let loaded_tip_bytes = loaded_host
        .engine
        .state_digest()
        .map_err(|err| tip_err(tip_turn, "", err, "failed to digest loaded tip state"))?;
    let expected_state_digest = hex::encode(replayed_tip_bytes);
    let loaded_tip_hex = hex::encode(loaded_tip_bytes);
    if expected_state_digest != loaded_tip_hex {
        return Err(tip_err(
            tip_turn,
            &expected_state_digest,
            loaded_tip_hex,
            "replayed tip state_digest != loaded tip state_digest",
        ));
    }
    let replayed_engine_bytes = serde_json::to_vec(&replayed_engine).map_err(|err| {
        tip_err(
            tip_turn,
            &expected_state_digest,
            err,
            "failed to serialize replayed TurnEngine",
        )
    })?;
    let loaded_engine_bytes = serde_json::to_vec(&loaded_host.engine).map_err(|err| {
        tip_err(
            tip_turn,
            &expected_state_digest,
            err,
            "failed to serialize loaded TurnEngine",
        )
    })?;
    if replayed_engine_bytes != loaded_engine_bytes {
        return Err(tip_err(
            tip_turn,
            &expected_state_digest,
            sha256_bytes_hex(&loaded_engine_bytes),
            "replayed TurnEngine bytes != loaded TurnEngine bytes",
        ));
    }

    let replayed_host = EvalHost {
        game_id: loaded_host.game_id.clone(),
        layout_seed: loaded_host.layout_seed,
        bot_tie_seed: loaded_host.bot_tie_seed,
        vrf_proofs: loaded_host.vrf_proofs.clone(),
        engine: replayed_engine,
        sealed_mandate_deal: loaded_host.sealed_mandate_deal.clone(),
        mandate_commits: loaded_host.mandate_commits.clone(),
        mandate_reveals: loaded_host.mandate_reveals.clone(),
        logged_turns: loaded_host.logged_turns.clone(),
        live_state_digests: loaded_host.live_state_digests.clone(),
        current_market_actions: Vec::new(),
        current_cancel_lock_at: None,
        current_commitments: BTreeMap::new(),
        current_reveals: BTreeMap::new(),
        turn_salts: loaded_host.turn_salts.clone(),
        recorded_act_submissions: BTreeMap::new(),
        active_pre_act_checkpoint_sha256: loaded_host.active_pre_act_checkpoint_sha256.clone(),
        engine_commit: loaded_host.engine_commit.clone(),
    };
    let mut replayed_probe = replayed_host;
    let mut loaded_probe = loaded_host.clone();
    if let Some(hook) = probe_fault_hook {
        hook(&mut loaded_probe);
    }

    let (replayed_post_digest, replayed_post_bytes) = execute_probe_step(&mut replayed_probe)
        .map_err(|detail| CheckpointVerifyError::ProbeStepDivergence {
            turn: tip_turn,
            replayed_digest: detail,
            loaded_digest: String::new(),
        })?;
    let (loaded_post_digest, loaded_post_bytes) =
        execute_probe_step(&mut loaded_probe).map_err(|detail| {
            CheckpointVerifyError::ProbeStepDivergence {
                turn: tip_turn,
                replayed_digest: replayed_post_digest.clone(),
                loaded_digest: detail,
            }
        })?;
    if replayed_post_digest != loaded_post_digest || replayed_post_bytes != loaded_post_bytes {
        return Err(CheckpointVerifyError::ProbeStepDivergence {
            turn: tip_turn,
            replayed_digest: replayed_post_digest,
            loaded_digest: loaded_post_digest,
        });
    }

    let summary = VerifiedCheckpointCausalSummary {
        pre_act_pair_sha256,
        pre_act_save_sha256,
        pre_act_seal_sha256,
        engine_commit,
        turn: tip_turn,
        seat_count: loaded_host.seat_count(),
        expected_state_digest,
        turn_chain_digest,
        post_probe_state_digest: replayed_post_digest,
    };
    Ok((loaded_host, summary))
}

/// Gate 2 causal verifier run at study preflight on a `.save.json` / `.seal` checkpoint pair.
///
/// Replays from Turn 0 through `turn - 1` inside the current engine, checks every per-turn digest,
/// verifies tip equivalence, and executes a 1-turn `ClubBot` probe step on both replayed and
/// loaded instances. `seal.engine_commit` is validated as 40 lowercase hex and copied into
/// [`VerifiedCheckpointCausalSummary::engine_commit`] as provenance without equality gating.
pub fn verify_checkpoint_causal(
    save_bytes: &[u8],
    seal_bytes: &[u8],
) -> Result<(EvalHost, VerifiedCheckpointCausalSummary), CheckpointVerifyError> {
    verify_checkpoint_causal_inner(save_bytes, seal_bytes, None, None)
}

/// Gate 2 causal verifier checking cumulative chain digest `C_{turn - 1}` against an expected
/// reference digest, raising [`CheckpointVerifyError::ChainHashMismatch`] on divergence (`M06`).
pub fn verify_checkpoint_causal_against_chain(
    save_bytes: &[u8],
    seal_bytes: &[u8],
    expected_turn_chain_digest: &str,
) -> Result<(EvalHost, VerifiedCheckpointCausalSummary), CheckpointVerifyError> {
    verify_checkpoint_causal_inner(
        save_bytes,
        seal_bytes,
        Some(expected_turn_chain_digest),
        None,
    )
}

/// Test-only probe divergence fault hook exercising [`CheckpointVerifyError::ProbeStepDivergence`].
#[doc(hidden)]
pub fn verify_checkpoint_causal_with_probe_fault_for_test(
    save_bytes: &[u8],
    seal_bytes: &[u8],
    fault_hook: fn(&mut EvalHost),
) -> Result<(EvalHost, VerifiedCheckpointCausalSummary), CheckpointVerifyError> {
    verify_checkpoint_causal_inner(save_bytes, seal_bytes, None, Some(fault_hook))
}

/// Gate 3 single-read worker checkpoint verifier and host constructor.
///
/// Checks outer save, seal, and pair SHA-256 digests (`OuterFileHashMismatch`), decodes strictly
/// (`decode_strict_checkpoint_pair`), and optionally validates `turn`, `seat_count`, and canonical
/// tip `state_digest` against the preflight manifest entry (`TipStateDigestMismatch`).
pub fn verify_and_construct_pre_act_bytes(
    save_bytes: &[u8],
    seal_bytes: &[u8],
    expected_save_sha256: Option<&str>,
    expected_seal_sha256: Option<&str>,
    expected_pair_sha256: Option<&str>,
    expected_manifest_tip: Option<(u8, u8, &str)>,
) -> Result<EvalHost, CheckpointVerifyError> {
    let actual_save = sha256_bytes_hex(save_bytes);
    if let Some(expected_save) = expected_save_sha256
        && actual_save != expected_save
    {
        return Err(CheckpointVerifyError::OuterFileHashMismatch {
            file: "save".to_owned(),
            expected: expected_save.to_owned(),
            actual: actual_save,
        });
    }
    let actual_seal = sha256_bytes_hex(seal_bytes);
    if let Some(expected_seal) = expected_seal_sha256
        && actual_seal != expected_seal
    {
        return Err(CheckpointVerifyError::OuterFileHashMismatch {
            file: "seal".to_owned(),
            expected: expected_seal.to_owned(),
            actual: actual_seal,
        });
    }
    let actual_pair = compute_pre_act_pair_sha256_hex(save_bytes, seal_bytes);
    if let Some(expected_pair) = expected_pair_sha256
        && actual_pair != expected_pair
    {
        return Err(CheckpointVerifyError::OuterFileHashMismatch {
            file: "pair".to_owned(),
            expected: expected_pair.to_owned(),
            actual: actual_pair,
        });
    }

    let host = decode_strict_checkpoint_pair(save_bytes, seal_bytes)?;
    if let Some((expected_turn, expected_seat_count, expected_state_digest)) = expected_manifest_tip
    {
        if host.turn() != expected_turn {
            return Err(tip_err(
                host.turn(),
                &expected_turn.to_string(),
                host.turn(),
                "loaded host turn != verified manifest turn",
            ));
        }
        if host.seat_count() != expected_seat_count {
            return Err(tip_err(
                host.turn(),
                &expected_seat_count.to_string(),
                host.seat_count(),
                "loaded host seat_count != verified manifest seat_count",
            ));
        }
        let actual_digest = host.canonical_state_digest_hex().map_err(|err| {
            tip_err(
                host.turn(),
                expected_state_digest,
                err,
                "failed to compute canonical state digest after load",
            )
        })?;
        if actual_digest != expected_state_digest {
            return Err(tip_err(
                host.turn(),
                expected_state_digest,
                actual_digest,
                "post-load canonical state_digest != verified manifest expected_state_digest",
            ));
        }
        if expected_turn > 1
            && host.last_live_state_digest_hex().as_deref() != Some(expected_state_digest)
        {
            return Err(tip_err(
                host.turn(),
                expected_state_digest,
                host.last_live_state_digest_hex().unwrap_or_default(),
                "last live_state_digest != verified manifest expected_state_digest",
            ));
        }
    }

    Ok(host)
}

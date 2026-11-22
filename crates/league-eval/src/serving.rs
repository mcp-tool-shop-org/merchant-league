//! Serving-stack manifest validation, startup Andon gates, wording digests, and environment
//! snapshot capture for reproducible Evaluation Mode runs.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use league_core::scrub_secret_text;
use league_relay::{
    ACTIVE_WORDING_ARM, CONTROL_ARM_WORDING, CONTROL_WORDING_ARM, MODEL_FACING_WORDING,
    PREVIEW_ARM_WORDING_R2, PREVIEW_WORDING_ARM_R2, WordingEntry, sha256_hex,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::EvalError;
use crate::wording::{EVAL_REPLY_WORDING, EVAL_REPLY_WORDING_ARM};

const HOME_PLACEHOLDER: &str = "<home>";
const REDACTED_TOKEN: &str = "[REDACTED]";
const SERVING_ENV_PREFIXES: [&str; 4] = ["LLAMA_", "CUDA_", "GGML_", "VLLM_"];
const SERVING_ENV_SECRET_SUBSTRINGS: [&str; 5] = ["KEY", "TOKEN", "SECRET", "PASS", "AUTH"];

/// Build-time git commit SHA compiled into `league-eval` (`build.rs`).
pub const CRATE_GIT_SHA: &str = env!("LEAGUE_EVAL_GIT_SHA");

/// Provenance tag distinguishing server-reported metadata from operator-supplied metadata.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataProvenance {
    /// Reported directly by the running inference server (for example via `/props` or `/v1/models`).
    ServerReported,
    /// Supplied by the operator / launch wrapper when server introspection is unavailable.
    #[default]
    OperatorSupplied,
}

/// Supported inference serving backend kind recorded in manifests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    /// `llama.cpp` `llama-server`.
    #[serde(alias = "llama-server", alias = "llama_cpp", alias = "llama.cpp")]
    LlamaServer,
    /// `vLLM` OpenAI-compatible server.
    Vllm,
}

/// A value tagged with whether it came from the server or the operator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceTagged<T> {
    /// Recorded value.
    pub value: T,
    /// Origin provenance (`server_reported` or `operator_supplied`).
    pub provenance: MetadataProvenance,
}

/// Raw model metadata inside an operator `--serving-manifest` JSON file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelManifestInput {
    /// Model filename (for example `"Qwen3-32B-Q4_K_M.gguf"`).
    pub filename: String,
    /// Quantization scheme (for example `"Q4_K_M"`, `"BF16"`, or `"FP8"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quant: Option<String>,
    /// Lowercase 64-hex SHA-256 of the model weights file (required).
    pub sha256: String,
    /// Optional 64-hex SHA-256 of the active chat template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_template_sha256: Option<String>,
    /// Provenance of model/template fields.
    #[serde(default)]
    pub provenance: MetadataProvenance,
}

/// Recorded model and template digests in `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelManifestRecord {
    /// Model filename.
    pub filename: String,
    /// Quantization label, if supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quant: Option<String>,
    /// Validated 64-hex SHA-256 of the model weights file.
    pub sha256: String,
    /// Validated 64-hex SHA-256 of the active chat template, if supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_template_sha256: Option<String>,
    /// Provenance of model/template fields.
    pub provenance: MetadataProvenance,
}

/// Raw hardware metadata inside an operator `--serving-manifest` JSON file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineManifestInput {
    /// GPU product name (required non-empty).
    pub gpu_name: String,
    /// Raw GPU UUID from the operator (hashed immediately and never written to disk).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_uuid: Option<String>,
    /// Pre-computed 64-hex SHA-256 of the GPU UUID (used if raw `gpu_uuid` is omitted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_uuid_sha256: Option<String>,
    /// Streaming multiprocessor count, if reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sm_count: Option<u32>,
    /// GPU driver version string.
    pub gpu_driver: String,
    /// CUDA runtime version (for NVIDIA GPUs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuda_runtime_version: Option<String>,
    /// ROCm version (when applicable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rocm_version: Option<String>,
    /// GPU VRAM capacity in MiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_mib: Option<u64>,
    /// Host OS version / build identifier.
    pub os_build: String,
    /// WSL kernel version when running under Windows Subsystem for Linux.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wsl_kernel_version: Option<String>,
    /// Provenance of machine hardware fields.
    #[serde(default)]
    pub provenance: MetadataProvenance,
}

/// Recorded machine metadata in `manifest.json` (contains `gpu_uuid_sha256` only, never raw UUID).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineManifestRecord {
    /// GPU product name.
    pub gpu_name: String,
    /// 64-hex SHA-256 digest of the GPU UUID.
    pub gpu_uuid_sha256: String,
    /// Streaming multiprocessor count, if supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sm_count: Option<u32>,
    /// GPU driver version string.
    pub gpu_driver: String,
    /// CUDA runtime version, if supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuda_runtime_version: Option<String>,
    /// ROCm version, if supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rocm_version: Option<String>,
    /// GPU VRAM capacity in MiB, if supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_mib: Option<u64>,
    /// Host OS version / build identifier.
    pub os_build: String,
    /// WSL kernel version, if running under WSL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wsl_kernel_version: Option<String>,
    /// Provenance of machine fields.
    pub provenance: MetadataProvenance,
}

/// Server execution settings input / record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSettingsRecord {
    /// Number of parallel server slots (must equal `1`).
    pub parallel_slots: u32,
    /// Logical prompt batch size (`-b`).
    pub batch_size: u32,
    /// Physical micro-batch size (`-ub`).
    pub ubatch_size: u32,
    /// Whether unified KV cache is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kv_unified: Option<bool>,
    /// Whether flash attention is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flash_attn: Option<bool>,
    /// K-cache quantization type (for example `"f16"`, `"q8_0"`).
    pub cache_type_k: String,
    /// V-cache quantization type.
    pub cache_type_v: String,
    /// Speculative decoding method (for example `None` or `"none"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speculative_decoding: Option<String>,
    /// Server context window length (`-c`).
    pub ctx_size: u32,
    /// Whether context shifting is enabled.
    pub ctx_shift: bool,
    /// Worker CPU thread count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threads: Option<u32>,
    /// Provenance of server execution settings.
    #[serde(default)]
    pub provenance: MetadataProvenance,
}

/// Prompt-cache controls supplied in `--serving-manifest` and checked against engine Andons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheSettingsInput {
    /// Per-request `cache_prompt` setting (`false` enforced on Local eval requests; must not be `true`).
    #[serde(default)]
    pub per_request_cache_prompt: Option<bool>,
    /// Whether the server process was also launched with `--no-cache-prompt`.
    #[serde(default)]
    pub no_cache_prompt_flag: bool,
    /// Host-RAM prompt cache size in MiB (`--cache-ram`; must be explicitly `0` for `llama-server`).
    #[serde(default)]
    pub cache_ram_mib: Option<i64>,
    /// Whether idle slot state saving is enabled (`--no-cache-idle-slots` => `false`; must be `false`).
    #[serde(default)]
    pub cache_idle_slots: Option<bool>,
    /// For vLLM: whether automatic prefix caching (`--enable-prefix-caching`) is on (must be `false`).
    #[serde(default)]
    pub enable_prefix_caching: Option<bool>,
    /// Provenance of cache configuration fields.
    #[serde(default)]
    pub provenance: MetadataProvenance,
}

/// Verified prompt-cache settings recorded in `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheSettingsRecord {
    /// Whether per-request `cache_prompt: false` was verified (`false` = disabled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_request_cache_prompt: Option<bool>,
    /// Whether the server process was also launched with `--no-cache-prompt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_cache_prompt_flag: Option<bool>,
    /// Recorded `--cache-ram` value in MiB (`0` when disabled on `llama-server`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_ram_mib: Option<i64>,
    /// Whether idle-slot saving is active (`false` = disabled via `--no-cache-idle-slots`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_idle_slots: Option<bool>,
    /// For vLLM: automatic prefix caching status (`false` = disabled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_prefix_caching: Option<bool>,
    /// Provenance of cache fields.
    pub provenance: MetadataProvenance,
}

/// Complete effective sampling parameters recorded in `manifest.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectiveSamplingRecord {
    /// Sampling temperature.
    pub temperature: f64,
    /// Top-k cutoff (`0` disables top-k).
    pub top_k: i32,
    /// Nucleus top-p cutoff.
    pub top_p: f64,
    /// Min-p threshold (including server defaults such as `0.05` when reported).
    pub min_p: f64,
    /// Repetition penalty (`1.0` disables).
    pub repeat_penalty: f64,
    /// Presence penalty (`0.0` disables).
    pub presence_penalty: f64,
    /// Frequency penalty (`0.0` disables).
    pub frequency_penalty: f64,
    /// Server / request RNG seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    /// Active stop sequences.
    #[serde(default)]
    pub stop_sequences: Vec<String>,
    /// Maximum generated tokens (`max_tokens`).
    pub max_tokens: u32,
    /// Effective sampler execution order reported or configured on the server.
    #[serde(default)]
    pub sampler_order: Vec<String>,
    /// Provenance of sampling parameters.
    #[serde(default)]
    pub provenance: MetadataProvenance,
}

/// Input JSON schema read by `league-eval` from `--serving-manifest <path>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServingManifestInput {
    /// Serving backend engine kind (`llama_server` or `vllm`).
    pub engine: EngineKind,
    /// Server version string (for example `"b11433"`).
    pub server_version: String,
    /// Optional server VCS build commit SHA.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_build_commit: Option<String>,
    /// Provenance of `engine` / `server_version` / `server_build_commit`.
    #[serde(default)]
    pub server_info_provenance: MetadataProvenance,
    /// Model filename, quant, SHA-256, and template hash.
    pub model: ModelManifestInput,
    /// Machine GPU/driver/CUDA/OS/WSL metadata.
    pub machine: MachineManifestInput,
    /// Server batching, slot, KV, and context settings.
    pub server_settings: ServerSettingsRecord,
    /// Prompt-cache settings audited at startup.
    pub cache_settings: CacheSettingsInput,
    /// Full effective sampling parameters.
    pub sampling: EffectiveSamplingRecord,
}

/// Verified serving configuration stored inside Evaluation `manifest.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServingManifestRecord {
    /// Serving backend engine kind with provenance.
    pub engine: ProvenanceTagged<EngineKind>,
    /// Server version with provenance.
    pub server_version: ProvenanceTagged<String>,
    /// Optional server build commit with provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_build_commit: Option<ProvenanceTagged<String>>,
    /// Cold-restart reproducibility status (`"not shown bit-exact"` on vLLM until verified).
    pub determinism_status: String,
    /// Model weights and template digests.
    pub model: ModelManifestRecord,
    /// Machine metadata with hashed `gpu_uuid_sha256` (raw `gpu_uuid` excluded).
    pub machine: MachineManifestRecord,
    /// Server batching, slot (`1`), KV, and context settings.
    pub server_settings: ServerSettingsRecord,
    /// Verified prefix-cache settings (all three `llama-server` layers off, or vLLM APC off).
    pub cache_settings: CacheSettingsRecord,
    /// Full effective sampler set and stops.
    pub sampling: EffectiveSamplingRecord,
}

/// Pinned SHA-256 digests of every wording table, active wrapper, and response schema in `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordingArmDigests {
    /// Neutral arm label (`v2_neutral_2026_11_15_r6`).
    pub neutral_arm: String,
    /// SHA-256 digest of canonical `MODEL_FACING_WORDING` entries.
    pub neutral_wording_sha256: String,
    /// Control arm label (`v2_control_2026_11_21_r1`).
    pub control_arm: String,
    /// SHA-256 digest of canonical `CONTROL_ARM_WORDING` entries.
    pub control_wording_sha256: String,
    /// Preview arm label (`v2_preview_2026_11_20_r2`).
    pub preview_arm: String,
    /// SHA-256 digest of canonical `PREVIEW_ARM_WORDING_R2` entries.
    pub preview_wording_sha256: String,
    /// Eval reply wording arm label (`v2_eval_reply_2026_11_21_r1`).
    pub eval_reply_arm: String,
    /// SHA-256 digest of canonical `EVAL_REPLY_WORDING` entries.
    pub eval_reply_wording_sha256: String,
    /// Active relay wording arm for this game or branch.
    pub active_relay_wording_arm: String,
    /// Active pinned wrapper version (`v2_seat_brief_2026_11_21_r3`).
    pub wrapper_version: String,
    /// Lowercase hex SHA-256 digest of the wrapper file bytes.
    pub wrapper_sha256: String,
    /// Lowercase hex SHA-256 digest of the unified `ResponseFormatConfig`.
    pub response_schema_sha256: String,
}

impl WordingArmDigests {
    /// Compute canonical wording, wrapper, and schema SHA-256 digests for manifest recording.
    #[must_use]
    pub fn compute(
        active_relay_wording_arm: &str,
        wrapper_version: &str,
        wrapper_sha256: &str,
        response_schema_sha256: &str,
    ) -> Self {
        Self {
            neutral_arm: ACTIVE_WORDING_ARM.to_owned(),
            neutral_wording_sha256: hash_wording_entries(MODEL_FACING_WORDING),
            control_arm: CONTROL_WORDING_ARM.to_owned(),
            control_wording_sha256: hash_wording_entries(CONTROL_ARM_WORDING),
            preview_arm: PREVIEW_WORDING_ARM_R2.to_owned(),
            preview_wording_sha256: hash_wording_entries(PREVIEW_ARM_WORDING_R2),
            eval_reply_arm: EVAL_REPLY_WORDING_ARM.to_owned(),
            eval_reply_wording_sha256: hash_wording_entries(EVAL_REPLY_WORDING),
            active_relay_wording_arm: active_relay_wording_arm.to_owned(),
            wrapper_version: wrapper_version.to_owned(),
            wrapper_sha256: wrapper_sha256.to_owned(),
            response_schema_sha256: response_schema_sha256.to_owned(),
        }
    }
}

fn hash_wording_entries(entries: &[WordingEntry]) -> String {
    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(entry.slot.as_bytes());
        hasher.update(b"\0");
        hasher.update(entry.shown_text.as_bytes());
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
}

fn is_valid_hex40(raw: &str) -> bool {
    raw.len() == 40 && raw.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_valid_hex64(raw: &str) -> bool {
    raw.len() == 64 && raw.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn normalize_hex64(raw: &str) -> Option<String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    is_valid_hex64(&trimmed).then_some(trimmed)
}

/// Validate `built_sha` (no `"unknown"`, must be 40 lowercase hex chars) and compare against
/// `live_head` when running inside a git checkout (`INVALID_CRATE_GIT_SHA`).
pub fn validate_crate_git_sha_against(
    built_sha: &str,
    live_head: Option<&str>,
) -> Result<String, EvalError> {
    let normalized = built_sha.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized == "unknown" || !is_valid_hex40(&normalized) {
        return Err(EvalError::InvalidCrateGitSha(
            "build revision `LEAGUE_EVAL_GIT_SHA` is missing, unknown, or malformed; rebuild cleanly inside git checkout".to_owned(),
        ));
    }
    if let Some(head_raw) = live_head {
        let head = head_raw.trim().to_ascii_lowercase();
        if !is_valid_hex40(&head) || head != normalized {
            return Err(EvalError::InvalidCrateGitSha(
                "stale `league-eval` build revision relative to current git HEAD; rebuild before running".to_owned(),
            ));
        }
    }
    Ok(normalized)
}

fn has_git_ancestor(dir: &Path) -> bool {
    dir.ancestors().any(|anc| anc.join(".git").exists())
}

/// Verify compiled [`CRATE_GIT_SHA`] is valid 40-hex (never `"unknown"`) and not stale relative to
/// the source checkout's current git `HEAD`.
pub fn resolve_verified_crate_git_sha() -> Result<String, EvalError> {
    let manifest_dir = option_env!("CARGO_MANIFEST_DIR").map(Path::new);
    let live_head = if let Some(dir) = manifest_dir
        && dir.is_dir()
        && has_git_ancestor(dir)
    {
        let out = Command::new("git")
            .current_dir(dir)
            .args(["rev-parse", "HEAD"])
            .output()
            .map_err(|e| {
                EvalError::InvalidCrateGitSha(format!("unable to inspect git HEAD: {e}"))
            })?;
        if !out.status.success() {
            return Err(EvalError::InvalidCrateGitSha(
                "git `rev-parse HEAD` failed while verifying `crate_git_sha`".to_owned(),
            ));
        }
        let head_str = String::from_utf8(out.stdout).map_err(|_| {
            EvalError::InvalidCrateGitSha("non-UTF-8 output from git `rev-parse HEAD`".to_owned())
        })?;
        Some(head_str)
    } else {
        None
    };
    validate_crate_git_sha_against(CRATE_GIT_SHA, live_head.as_deref())
}

/// Validate a [`ServingManifestInput`] against all startup reproducibility Andon rules and produce a
/// sanitized [`ServingManifestRecord`] (hashing raw `gpu_uuid` and scrubbing all free-text fields).
pub fn validate_serving_manifest_input(
    input: &ServingManifestInput,
) -> Result<ServingManifestRecord, EvalError> {
    let homes = detect_process_home_roots();
    if input.server_version.trim().is_empty() {
        return Err(EvalError::MissingServingManifest(
            "serving manifest requires a non-empty `server_version`".to_owned(),
        ));
    }
    if input.model.filename.trim().is_empty() {
        return Err(EvalError::MissingModelSha256(
            "serving manifest requires a non-empty `model.filename`".to_owned(),
        ));
    }
    let model_sha256 = normalize_hex64(&input.model.sha256).ok_or_else(|| {
        EvalError::MissingModelSha256(
            "`model.sha256` must be a non-empty 64-hex SHA-256 digest".to_owned(),
        )
    })?;
    let chat_template_sha256 = match &input.model.chat_template_sha256 {
        None => None,
        Some(raw) => Some(normalize_hex64(raw).ok_or_else(|| {
            EvalError::MissingServingManifest(
                "`model.chat_template_sha256` must be 64 lowercase hex characters when set"
                    .to_owned(),
            )
        })?),
    };

    if input.machine.gpu_name.trim().is_empty() {
        return Err(EvalError::MissingGpuName(
            "serving manifest requires a non-empty `machine.gpu_name`".to_owned(),
        ));
    }
    let gpu_uuid_sha256 = match (
        input
            .machine
            .gpu_uuid
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        input.machine.gpu_uuid_sha256.as_deref(),
    ) {
        (Some(raw_uuid), None) => sha256_hex(raw_uuid.as_bytes()),
        (None, Some(prehashed)) => normalize_hex64(prehashed).ok_or_else(|| {
            EvalError::InvalidGpuUuidSha256(
                "`machine.gpu_uuid_sha256` must be 64 hex characters".to_owned(),
            )
        })?,
        (Some(raw_uuid), Some(prehashed)) => {
            let expected = sha256_hex(raw_uuid.as_bytes());
            let supplied = normalize_hex64(prehashed).ok_or_else(|| {
                EvalError::InvalidGpuUuidSha256(
                    "`machine.gpu_uuid_sha256` must be 64 hex characters".to_owned(),
                )
            })?;
            if expected != supplied {
                return Err(EvalError::InvalidGpuUuidSha256(
                    "supplied `machine.gpu_uuid` hash disagrees with `machine.gpu_uuid_sha256`"
                        .to_owned(),
                ));
            }
            expected
        }
        (None, None) => {
            return Err(EvalError::InvalidGpuUuidSha256(
                "serving manifest requires `machine.gpu_uuid` or `machine.gpu_uuid_sha256`"
                    .to_owned(),
            ));
        }
    };

    if input.server_settings.parallel_slots != 1 {
        return Err(EvalError::ParallelSlotsExceeded(format!(
            "`server_settings.parallel_slots` must equal 1 for reproducible runs, got {}",
            input.server_settings.parallel_slots
        )));
    }

    let (determinism_status, cache_record) = match input.engine {
        EngineKind::LlamaServer => {
            if input.cache_settings.per_request_cache_prompt != Some(false) {
                return Err(EvalError::PrefixCacheEnabled(
                    "llama-server requires `cache_settings.per_request_cache_prompt: false` (`cache_prompt: false` on every request)".to_owned(),
                ));
            }
            if input.cache_settings.cache_ram_mib != Some(0) {
                return Err(EvalError::PrefixCacheEnabled(format!(
                    "llama-server host-RAM prompt cache must be disabled (`--cache-ram 0` -> `cache_ram_mib: 0`), got {:?}",
                    input.cache_settings.cache_ram_mib
                )));
            }
            if input.cache_settings.cache_idle_slots != Some(false) {
                return Err(EvalError::PrefixCacheEnabled(format!(
                    "llama-server idle-slot saving must be disabled (`--no-cache-idle-slots` -> `cache_idle_slots: false`), got {:?}",
                    input.cache_settings.cache_idle_slots
                )));
            }
            (
                "llama_server_cache_disabled".to_owned(),
                CacheSettingsRecord {
                    per_request_cache_prompt: Some(false),
                    no_cache_prompt_flag: Some(input.cache_settings.no_cache_prompt_flag),
                    cache_ram_mib: Some(0),
                    cache_idle_slots: Some(false),
                    enable_prefix_caching: None,
                    provenance: input.cache_settings.provenance,
                },
            )
        }
        EngineKind::Vllm => {
            if input.cache_settings.enable_prefix_caching != Some(false) {
                return Err(EvalError::PrefixCacheEnabled(format!(
                    "vLLM requires `cache_settings.enable_prefix_caching: false` (`--no-enable-prefix-caching`), got {:?}",
                    input.cache_settings.enable_prefix_caching
                )));
            }
            (
                "not shown bit-exact".to_owned(),
                CacheSettingsRecord {
                    per_request_cache_prompt: None,
                    no_cache_prompt_flag: None,
                    cache_ram_mib: None,
                    cache_idle_slots: None,
                    enable_prefix_caching: Some(false),
                    provenance: input.cache_settings.provenance,
                },
            )
        }
    };

    let server_version = scrub_manifest_field("server_version", &input.server_version, &homes)?;
    let server_build_commit = input
        .server_build_commit
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .map(|c| {
            Ok::<_, EvalError>(ProvenanceTagged {
                value: scrub_manifest_field("server_build_commit", c, &homes)?,
                provenance: input.server_info_provenance,
            })
        })
        .transpose()?;

    let mut clean_server_settings = input.server_settings.clone();
    clean_server_settings.cache_type_k = scrub_manifest_field(
        "server_settings.cache_type_k",
        &input.server_settings.cache_type_k,
        &homes,
    )?;
    clean_server_settings.cache_type_v = scrub_manifest_field(
        "server_settings.cache_type_v",
        &input.server_settings.cache_type_v,
        &homes,
    )?;
    clean_server_settings.speculative_decoding = input
        .server_settings
        .speculative_decoding
        .as_deref()
        .map(|s| scrub_manifest_field("server_settings.speculative_decoding", s, &homes))
        .transpose()?;

    let mut clean_sampling = input.sampling.clone();
    clean_sampling.stop_sequences = input
        .sampling
        .stop_sequences
        .iter()
        .enumerate()
        .map(|(i, s)| scrub_manifest_field(&format!("sampling.stop_sequences[{i}]"), s, &homes))
        .collect::<Result<Vec<_>, _>>()?;
    clean_sampling.sampler_order = input
        .sampling
        .sampler_order
        .iter()
        .enumerate()
        .map(|(i, s)| scrub_manifest_field(&format!("sampling.sampler_order[{i}]"), s, &homes))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ServingManifestRecord {
        engine: ProvenanceTagged {
            value: input.engine,
            provenance: input.server_info_provenance,
        },
        server_version: ProvenanceTagged {
            value: server_version,
            provenance: input.server_info_provenance,
        },
        server_build_commit,
        determinism_status,
        model: ModelManifestRecord {
            filename: scrub_manifest_field("model.filename", &input.model.filename, &homes)?,
            quant: input
                .model
                .quant
                .as_deref()
                .map(|q| scrub_manifest_field("model.quant", q, &homes))
                .transpose()?,
            sha256: model_sha256,
            chat_template_sha256,
            provenance: input.model.provenance,
        },
        machine: MachineManifestRecord {
            gpu_name: scrub_manifest_field("machine.gpu_name", &input.machine.gpu_name, &homes)?,
            gpu_uuid_sha256,
            sm_count: input.machine.sm_count,
            gpu_driver: scrub_manifest_field(
                "machine.gpu_driver",
                &input.machine.gpu_driver,
                &homes,
            )?,
            cuda_runtime_version: input
                .machine
                .cuda_runtime_version
                .as_deref()
                .map(|v| scrub_manifest_field("machine.cuda_runtime_version", v, &homes))
                .transpose()?,
            rocm_version: input
                .machine
                .rocm_version
                .as_deref()
                .map(|v| scrub_manifest_field("machine.rocm_version", v, &homes))
                .transpose()?,
            vram_mib: input.machine.vram_mib,
            os_build: scrub_manifest_field("machine.os_build", &input.machine.os_build, &homes)?,
            wsl_kernel_version: input
                .machine
                .wsl_kernel_version
                .as_deref()
                .map(|v| scrub_manifest_field("machine.wsl_kernel_version", v, &homes))
                .transpose()?,
            provenance: input.machine.provenance,
        },
        server_settings: clean_server_settings,
        cache_settings: cache_record,
        sampling: clean_sampling,
    })
}

/// Load a JSON file at `path` and validate it via [`validate_serving_manifest_input`].
pub fn load_and_validate_serving_manifest(path: &Path) -> Result<ServingManifestRecord, EvalError> {
    if !path.exists() {
        return Err(EvalError::MissingServingManifest(format!(
            "serving manifest file not found at {}",
            path.display()
        )));
    }
    let raw = fs::read_to_string(path).map_err(|e| {
        EvalError::MissingServingManifest(format!(
            "cannot read serving manifest {}: {e}",
            path.display()
        ))
    })?;
    let parsed: ServingManifestInput = serde_json::from_str(&raw).map_err(|e| {
        EvalError::MissingServingManifest(format!(
            "invalid serving manifest JSON in {} at line {}, column {}",
            path.display(),
            e.line(),
            e.column()
        ))
    })?;
    validate_serving_manifest_input(&parsed)
}

fn detect_process_home_roots() -> Vec<PathBuf> {
    let mut homes = Vec::new();
    for var in ["HOME", "USERPROFILE"] {
        if let Some(val) = std::env::var_os(var)
            && !val.is_empty()
        {
            homes.push(PathBuf::from(val));
        }
    }
    homes
}

/// Capture serving-related OS environment variables (`LLAMA_*`, `CUDA_*`, `GGML_*`, `VLLM_*`),
/// omitting secret-named variables, hashing `CUDA_VISIBLE_DEVICES` and any GPU/MIG IDs under
/// `<key>_sha256`, replacing home paths with `<home>`, and scrubbing any token patterns.
pub fn capture_serving_env_snapshot() -> Result<BTreeMap<String, String>, EvalError> {
    let homes = detect_process_home_roots();
    capture_serving_env_snapshot_from(std::env::vars(), &homes)
}

/// Pure testable core of [`capture_serving_env_snapshot`] over explicit env vars and home roots.
pub fn capture_serving_env_snapshot_from<I>(
    vars: I,
    homes: &[PathBuf],
) -> Result<BTreeMap<String, String>, EvalError>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut out = BTreeMap::new();
    for (name, value) in vars {
        let upper = name.to_ascii_uppercase();
        if !SERVING_ENV_PREFIXES
            .iter()
            .any(|prefix| upper.starts_with(prefix))
        {
            continue;
        }
        if SERVING_ENV_SECRET_SUBSTRINGS
            .iter()
            .any(|secret| upper.contains(secret))
        {
            continue;
        }
        if upper == "CUDA_VISIBLE_DEVICES" || contains_gpu_or_mig_id(&value) {
            out.insert(format!("{name}_sha256"), sha256_hex(value.as_bytes()));
            continue;
        }
        let cleaned = scrub_text_value(&format!("env.{name}"), &value, homes, false)?;
        out.insert(name, cleaned);
    }
    Ok(out)
}

fn scrub_manifest_field(field: &str, value: &str, homes: &[PathBuf]) -> Result<String, EvalError> {
    scrub_text_value(field, value, homes, true)
}

fn contains_unsafe_control_chars(value: &str) -> bool {
    value
        .bytes()
        .any(|b| b == 0 || (b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r')) || b == 0x7f)
}

fn contains_gpu_or_mig_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    let lower = value.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let mut i = 0usize;
    while i < lb.len() {
        let prev_boundary = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if prev_boundary && lb[i..].starts_with(b"gpu-") {
            let body = &bytes[i + 4..];
            let mut end = 0usize;
            while end < body.len() && (body[end].is_ascii_hexdigit() || body[end] == b'-') {
                end += 1;
            }
            while end > 0 && body[end - 1] == b'-' {
                end -= 1;
            }
            let seg = &body[..end];
            let hex_digits = seg.iter().filter(|b| b.is_ascii_hexdigit()).count();
            let dec_digits = seg.iter().filter(|b| b.is_ascii_digit()).count();
            if seg.len() >= 8 && hex_digits >= 4 && dec_digits >= 1 {
                return true;
            }
        } else if prev_boundary && lb[i..].starts_with(b"mig-") {
            let body = &lb[i + 4..];
            let raw_body = &bytes[i + 4..];
            let scan_from = if body.starts_with(b"gpu-") { 4 } else { 0 };
            let mut end = scan_from;
            while end < raw_body.len()
                && (raw_body[end].is_ascii_hexdigit() || matches!(raw_body[end], b'-' | b'/'))
            {
                end += 1;
            }
            while end > scan_from && matches!(raw_body[end - 1], b'-' | b'/') {
                end -= 1;
            }
            let seg = &raw_body[scan_from..end];
            let hex_digits = seg.iter().filter(|b| b.is_ascii_hexdigit()).count();
            let dec_digits = seg.iter().filter(|b| b.is_ascii_digit()).count();
            if seg.len() >= 8 && hex_digits >= 4 && dec_digits >= 1 {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn scrub_text_value(
    field_or_key: &str,
    value: &str,
    homes: &[PathBuf],
    scrub_generic_homes: bool,
) -> Result<String, EvalError> {
    if contains_unsafe_control_chars(value) {
        return Err(EvalError::UnsafeManifestValue(format!(
            "field `{field_or_key}` contains unsupported control characters"
        )));
    }
    if contains_gpu_or_mig_id(value) {
        return Err(EvalError::UnsafeManifestValue(format!(
            "field `{field_or_key}` cannot contain a raw GPU or MIG identifier"
        )));
    }
    let mut s = replace_home_paths_in_value(value, homes);
    s = replace_wsl_users_home(&s);
    if scrub_generic_homes {
        s = replace_generic_user_homes(&s);
    }
    s = redact_token_patterns(&s);
    if has_residual_unsafe_value(&s, scrub_generic_homes) {
        return Err(EvalError::UnsafeManifestValue(format!(
            "field `{field_or_key}` contains an unsafe or un-scrubbable value"
        )));
    }
    Ok(s)
}

fn redact_token_patterns(input: &str) -> String {
    let base_scrubbed = scrub_secret_text(input);
    let mut out = String::with_capacity(base_scrubbed.len());
    let bytes = base_scrubbed.as_bytes();
    let lower = base_scrubbed.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let mut i = 0usize;

    while i < base_scrubbed.len() {
        let prev_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if prev_ok && lb[i..].starts_with(b"github_pat_") {
            let mut end = i + 11;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end - (i + 11) >= 10 {
                out.push_str(REDACTED_TOKEN);
                i = end;
                continue;
            }
        } else if prev_ok && (lb[i..].starts_with(b"ghp_") || lb[i..].starts_with(b"hf_")) {
            let plen = if lb[i..].starts_with(b"ghp_") { 4 } else { 3 };
            let mut end = i + plen;
            while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
                end += 1;
            }
            if end - (i + plen) >= 10 {
                out.push_str(REDACTED_TOKEN);
                i = end;
                continue;
            }
        } else if prev_ok && lb[i..].starts_with(b"sk-") {
            let mut end = i + 3;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'-' | b'_'))
            {
                end += 1;
            }
            if end - (i + 3) >= 8 {
                out.push_str(REDACTED_TOKEN);
                i = end;
                continue;
            }
        } else if prev_ok && lb[i..].starts_with(b"bearer") {
            let after_b = i + 6;
            if after_b < bytes.len()
                && (bytes[after_b].is_ascii_whitespace() || matches!(bytes[after_b], b':' | b'='))
            {
                let mut tok_start = after_b;
                while tok_start < bytes.len()
                    && (bytes[tok_start].is_ascii_whitespace()
                        || matches!(bytes[tok_start], b':' | b'='))
                {
                    tok_start += 1;
                }
                let mut tok_end = tok_start;
                while tok_end < bytes.len()
                    && (bytes[tok_end].is_ascii_alphanumeric()
                        || matches!(bytes[tok_end], b'.' | b'_' | b'-' | b'+' | b'/' | b'='))
                {
                    tok_end += 1;
                }
                if tok_end > tok_start && !base_scrubbed[tok_start..tok_end].starts_with('[') {
                    out.push_str(REDACTED_TOKEN);
                    i = tok_end;
                    continue;
                }
            }
        } else if prev_ok
            && let Some(eq_len) = ["api_key=", "token=", "secret=", "password=", "auth="]
                .iter()
                .find(|kw| lb[i..].starts_with(kw.as_bytes()))
                .map(|kw| kw.len())
        {
            let val_start = i + eq_len;
            if val_start < bytes.len() && !base_scrubbed[val_start..].starts_with(REDACTED_TOKEN) {
                let mut val_end = val_start;
                while val_end < bytes.len()
                    && !matches!(
                        bytes[val_end],
                        b' ' | b'\t' | b'\r' | b'\n' | b',' | b';' | b'"' | b'\'' | b')' | b']'
                    )
                {
                    val_end += 1;
                }
                if val_end > val_start {
                    out.push_str(&base_scrubbed[i..val_start]);
                    out.push_str(REDACTED_TOKEN);
                    i = val_end;
                    continue;
                }
            }
        }

        let Some(ch) = base_scrubbed[i..].chars().next() else {
            break;
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn has_residual_unsafe_value(scrubbed: &str, check_generic_homes: bool) -> bool {
    if contains_unsafe_control_chars(scrubbed) || contains_gpu_or_mig_id(scrubbed) {
        return true;
    }
    let bytes = scrubbed.as_bytes();
    let lower = scrubbed.to_ascii_lowercase();
    let lb = lower.as_bytes();
    for i in 0..bytes.len() {
        let prev_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if !prev_ok {
            continue;
        }
        if (lb[i..].starts_with(b"github_pat_")
            && bytes
                .get(i + 11..i + 13)
                .is_some_and(|s| s.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')))
            || (lb[i..].starts_with(b"ghp_")
                && bytes
                    .get(i + 4..i + 6)
                    .is_some_and(|s| s.iter().all(u8::is_ascii_alphanumeric)))
            || (lb[i..].starts_with(b"hf_")
                && bytes
                    .get(i + 3..i + 5)
                    .is_some_and(|s| s.iter().all(u8::is_ascii_alphanumeric)))
            || (lb[i..].starts_with(b"sk-")
                && bytes
                    .get(i + 3..i + 5)
                    .is_some_and(|s| s.iter().all(u8::is_ascii_alphanumeric)))
        {
            return true;
        }
        if lb[i..].starts_with(b"bearer")
            && let Some(sep) = bytes.get(i + 6)
            && (sep.is_ascii_whitespace() || matches!(sep, b':' | b'='))
        {
            let rest = scrubbed[i + 7..]
                .trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ':' || c == '=');
            if !rest.is_empty() && !rest.starts_with(REDACTED_TOKEN) {
                return true;
            }
        }
    }
    let prefix_ok = |idx: usize| {
        idx == 0
            || scrubbed[..idx]
                .chars()
                .next_back()
                .is_some_and(is_home_prefix_boundary)
    };
    for (idx, _) in lower.match_indices("/mnt/") {
        if prefix_ok(idx)
            && lb.get(idx + 5).is_some_and(u8::is_ascii_alphabetic)
            && lb.get(idx + 6..idx + 13) == Some(b"/users/")
        {
            return true;
        }
    }
    if check_generic_homes {
        for (idx, _) in scrubbed.match_indices("/home/") {
            if prefix_ok(idx)
                && scrubbed
                    .get(idx + 6..idx + 7)
                    .is_some_and(|s| s.bytes().all(|b| !matches!(b, b'/' | b'\\' | b' ')))
            {
                return true;
            }
        }
        for (idx, _) in lower.match_indices("/users/") {
            if prefix_ok(idx) {
                return true;
            }
        }
        for (idx, _) in lower.match_indices(":\\users\\") {
            if idx >= 1 && bytes[idx - 1].is_ascii_alphabetic() && prefix_ok(idx - 1) {
                return true;
            }
        }
    }
    false
}

fn is_home_prefix_boundary(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '=' | ':' | ';' | '"' | '\'' | ',' | '(' | '['
    )
}

fn is_home_suffix_boundary(c: char) -> bool {
    matches!(
        c,
        '/' | '\\' | ' ' | '\t' | ':' | ';' | '"' | '\'' | ',' | ')' | ']'
    )
}

fn replace_wsl_users_home(haystack: &str) -> String {
    let mut out = String::with_capacity(haystack.len());
    let bytes = haystack.as_bytes();
    let lower = haystack.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let mut i = 0usize;
    while i < haystack.len() {
        let prev_ok = i == 0
            || haystack[..i]
                .chars()
                .next_back()
                .is_some_and(is_home_prefix_boundary);
        if prev_ok
            && lb[i..].starts_with(b"/mnt/")
            && bytes.get(i + 5).is_some_and(u8::is_ascii_alphabetic)
            && lb.get(i + 6..i + 13) == Some(b"/users/")
        {
            let name_start = i + 13;
            let mut name_end = name_start;
            while name_end < haystack.len() {
                let Some(ch) = haystack[name_end..].chars().next() else {
                    break;
                };
                if is_home_suffix_boundary(ch) {
                    break;
                }
                name_end += ch.len_utf8();
            }
            let next_ok = name_end == haystack.len()
                || haystack[name_end..]
                    .chars()
                    .next()
                    .is_some_and(is_home_suffix_boundary);
            if name_end > name_start && next_ok {
                out.push_str(HOME_PLACEHOLDER);
                i = name_end;
                continue;
            }
        }
        let Some(ch) = haystack[i..].chars().next() else {
            break;
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn match_generic_home_end(haystack: &str, lower_bytes: &[u8], i: usize) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let name_start = if bytes[i..].starts_with(b"/home/") {
        i + 6
    } else if lower_bytes[i..].starts_with(b"/users/") {
        i + 7
    } else if bytes.get(i).is_some_and(u8::is_ascii_alphabetic)
        && (lower_bytes.get(i + 1..i + 9) == Some(b":\\users\\")
            || lower_bytes.get(i + 1..i + 9) == Some(b":/users/"))
    {
        i + 9
    } else {
        return None;
    };
    let mut name_end = name_start;
    while name_end < haystack.len() {
        let ch = haystack[name_end..].chars().next()?;
        if is_home_suffix_boundary(ch) {
            break;
        }
        name_end += ch.len_utf8();
    }
    let next_ok = name_end == haystack.len()
        || haystack[name_end..]
            .chars()
            .next()
            .is_some_and(is_home_suffix_boundary);
    (name_end > name_start && next_ok).then_some(name_end)
}

fn replace_generic_user_homes(haystack: &str) -> String {
    let mut out = String::with_capacity(haystack.len());
    let lower = haystack.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let mut i = 0usize;
    while i < haystack.len() {
        let prev_ok = i == 0
            || haystack[..i]
                .chars()
                .next_back()
                .is_some_and(is_home_prefix_boundary);
        if prev_ok && let Some(end) = match_generic_home_end(haystack, lb, i) {
            out.push_str(HOME_PLACEHOLDER);
            i = end;
            continue;
        }
        let Some(ch) = haystack[i..].chars().next() else {
            break;
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn replace_home_paths_in_value(value: &str, homes: &[PathBuf]) -> String {
    let mut patterns: Vec<String> = Vec::new();
    for home in homes {
        let raw = home.to_string_lossy();
        let trimmed_native = raw.trim_end_matches(['/', '\\']);
        if trimmed_native.is_empty() {
            continue;
        }
        patterns.push(trimmed_native.to_owned());
        let slash = trimmed_native.replace('\\', "/");
        if slash != trimmed_native {
            patterns.push(slash);
        }
    }
    patterns.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    patterns.dedup();

    let mut current = value.to_owned();
    for pat in patterns {
        current = replace_substring_with_home(&current, &pat);
    }
    current
}

fn replace_substring_with_home(haystack: &str, pattern: &str) -> String {
    #[cfg(windows)]
    let match_eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    #[cfg(not(windows))]
    let match_eq = |a: &str, b: &str| a == b;

    let mut out = String::with_capacity(haystack.len());
    let mut i = 0usize;
    let p_len = pattern.len();
    while i < haystack.len() {
        if haystack.is_char_boundary(i)
            && i + p_len <= haystack.len()
            && haystack.is_char_boundary(i + p_len)
            && match_eq(&haystack[i..i + p_len], pattern)
        {
            let prev_ok = i == 0
                || haystack[..i]
                    .chars()
                    .next_back()
                    .is_some_and(is_home_prefix_boundary);
            let next_ok = i + p_len == haystack.len()
                || haystack[i + p_len..]
                    .chars()
                    .next()
                    .is_some_and(is_home_suffix_boundary);
            if prev_ok && next_ok {
                out.push_str(HOME_PLACEHOLDER);
                i += p_len;
                continue;
            }
        }
        let Some(ch) = haystack[i..].chars().next() else {
            break;
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_snapshot_keeps_prefixed_strips_secrets_and_replaces_homes() {
        let homes = vec![
            PathBuf::from("/home/evaluator"),
            PathBuf::from(r"X:\Profiles\evaluator"),
        ];
        let vars = vec![
            (
                "LLAMA_CACHE".to_owned(),
                "/home/evaluator/.cache/llama".to_owned(),
            ),
            (
                "GGML_CUDA_PATH".to_owned(),
                r"X:\Profiles\evaluator\cuda\bin".to_owned(),
            ),
            ("CUDA_VISIBLE_DEVICES".to_owned(), "0".to_owned()),
            ("VLLM_ATTENTION_BACKEND".to_owned(), "FLASH_ATTN".to_owned()),
            ("VLLM_API_KEY".to_owned(), "sk-secret-key".to_owned()),
            ("LLAMA_HF_TOKEN".to_owned(), "hf_secret".to_owned()),
            (
                "CUDA_SECRET_CACHE".to_owned(),
                "/home/evaluator/secret".to_owned(),
            ),
            (
                "GGML_PASS_FILE".to_owned(),
                "/home/evaluator/pass".to_owned(),
            ),
            ("VLLM_AUTH_HEADER".to_owned(), "Bearer xyz".to_owned()),
            ("vllm_token_lower".to_owned(), "tok-lower".to_owned()),
            ("PATH".to_owned(), "/home/evaluator/bin:/usr/bin".to_owned()),
        ];

        let snap = capture_serving_env_snapshot_from(vars, &homes)
            .unwrap_or_else(|e| panic!("env snapshot failed: {e}"));
        assert_eq!(snap.len(), 4);
        assert_eq!(
            snap.get("LLAMA_CACHE").map(String::as_str),
            Some("<home>/.cache/llama")
        );
        assert_eq!(
            snap.get("GGML_CUDA_PATH").map(String::as_str),
            Some(r"<home>\cuda\bin")
        );
        assert_eq!(
            snap.get("CUDA_VISIBLE_DEVICES_sha256").map(String::as_str),
            Some(sha256_hex(b"0").as_str())
        );
        assert!(!snap.contains_key("CUDA_VISIBLE_DEVICES"));
        assert_eq!(
            snap.get("VLLM_ATTENTION_BACKEND").map(String::as_str),
            Some("FLASH_ATTN")
        );
        for forbidden_key in [
            "VLLM_API_KEY",
            "LLAMA_HF_TOKEN",
            "CUDA_SECRET_CACHE",
            "GGML_PASS_FILE",
            "VLLM_AUTH_HEADER",
            "vllm_token_lower",
            "PATH",
        ] {
            assert!(
                !snap.contains_key(forbidden_key),
                "unexpected key '{forbidden_key}' in {snap:?}"
            );
        }
        let rendered = serde_json::to_string(&snap).unwrap_or_default();
        for forbidden_text in [
            "VLLM_API_KEY",
            "LLAMA_HF_TOKEN",
            "CUDA_SECRET_CACHE",
            "GGML_PASS_FILE",
            "VLLM_AUTH_HEADER",
            "vllm_token_lower",
            "/home/evaluator",
            "evaluator",
            "sk-secret-key",
            "hf_secret",
            "/usr/bin",
        ] {
            assert!(
                !rendered.contains(forbidden_text),
                "unexpected leak '{forbidden_text}' in {rendered}"
            );
        }
    }

    #[test]
    fn fix2_s2_gpu_and_mig_ids_hashed_never_verbatim() {
        let gpu_val = [
            "GPU",
            "-",
            "1a2b3c4d",
            "-",
            "0000",
            "-",
            "1111",
            "-",
            "2222",
            "-",
            "deadbeef0001",
        ]
        .concat();
        let mig_val = ["MIG", "-", "a1b2c3d4", "-", "9999", "-", "0", "-", "1"].concat();
        let vars = vec![
            ("CUDA_VISIBLE_DEVICES".to_owned(), gpu_val.clone()),
            ("VLLM_DEVICE_ID".to_owned(), mig_val.clone()),
            (
                "VLLM_WORKER_MULTIPROC_METHOD".to_owned(),
                "spawn".to_owned(),
            ),
        ];
        let snap = capture_serving_env_snapshot_from(vars, &[])
            .unwrap_or_else(|e| panic!("unexpected snapshot error: {e}"));
        assert_eq!(
            snap.get("CUDA_VISIBLE_DEVICES_sha256").map(String::as_str),
            Some(sha256_hex(gpu_val.as_bytes()).as_str())
        );
        assert_eq!(
            snap.get("VLLM_DEVICE_ID_sha256").map(String::as_str),
            Some(sha256_hex(mig_val.as_bytes()).as_str())
        );
        assert_eq!(
            snap.get("VLLM_WORKER_MULTIPROC_METHOD").map(String::as_str),
            Some("spawn")
        );
        assert!(!snap.contains_key("CUDA_VISIBLE_DEVICES"));
        assert!(!snap.contains_key("VLLM_DEVICE_ID"));
        let rendered = serde_json::to_string(&snap).unwrap_or_default();
        assert!(!rendered.contains(&gpu_val));
        assert!(!rendered.contains(&mig_val));
    }

    #[test]
    fn fix2_s3_git_sha_rejects_unknown_and_stale_and_resolves_worktree() {
        for bad in ["unknown", "", "deadbeef", &"z".repeat(40)] {
            let err = match validate_crate_git_sha_against(bad, None) {
                Err(e) => e,
                Ok(v) => panic!("expected INVALID_CRATE_GIT_SHA for '{bad}', got {v}"),
            };
            assert_eq!(err.code(), "INVALID_CRATE_GIT_SHA");
        }
        let sha_a = "1".repeat(40);
        let sha_b = "2".repeat(40);
        let stale_err = match validate_crate_git_sha_against(&sha_a, Some(&sha_b)) {
            Err(e) => e,
            Ok(v) => panic!("expected INVALID_CRATE_GIT_SHA on stale head, got {v}"),
        };
        assert_eq!(stale_err.code(), "INVALID_CRATE_GIT_SHA");
        let verified = validate_crate_git_sha_against(&sha_a, Some(&sha_a))
            .unwrap_or_else(|e| panic!("matching 40-hex SHA should pass: {e}"));
        assert_eq!(verified, sha_a);

        let live = resolve_verified_crate_git_sha()
            .unwrap_or_else(|e| panic!("worktree CRATE_GIT_SHA must match HEAD: {e}"));
        assert_eq!(live.len(), 40);
        assert!(live.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(live, CRATE_GIT_SHA);
    }

    #[test]
    fn fix2_s6_home_boundaries_comma_paren_bracket_and_wsl_keep_probeuserx_untouched() {
        let homes = vec![
            PathBuf::from("/home/probeuser"),
            PathBuf::from(r"X:\Profiles\probeuser"),
        ];
        let vars = vec![
            (
                "CUDA_VISIBLE_COMMA".to_owned(),
                "/home/probeuser/cuda,other=/home/probeuser/lib".to_owned(),
            ),
            (
                "VLLM_OPT_PARENS".to_owned(),
                "(/home/probeuser/venv/bin)".to_owned(),
            ),
            (
                "VLLM_OPT_BRACKETS".to_owned(),
                r"[/home/probeuser/cache,X:\Profiles\probeuser\models]".to_owned(),
            ),
            (
                "LLAMA_WSL_HOME".to_owned(),
                "/mnt/c/Users/probeuser/models/q.gguf".to_owned(),
            ),
            (
                "VLLM_SIBLING_USER".to_owned(),
                "/home/probeuserX/y".to_owned(),
            ),
        ];
        let snap = capture_serving_env_snapshot_from(vars, &homes)
            .unwrap_or_else(|e| panic!("snapshot error: {e}"));
        assert_eq!(
            snap.get("CUDA_VISIBLE_COMMA").map(String::as_str),
            Some("<home>/cuda,other=<home>/lib")
        );
        assert_eq!(
            snap.get("VLLM_OPT_PARENS").map(String::as_str),
            Some("(<home>/venv/bin)")
        );
        assert_eq!(
            snap.get("VLLM_OPT_BRACKETS").map(String::as_str),
            Some(r"[<home>/cache,<home>\models]")
        );
        assert_eq!(
            snap.get("LLAMA_WSL_HOME").map(String::as_str),
            Some("<home>/models/q.gguf")
        );
        assert_eq!(
            snap.get("VLLM_SIBLING_USER").map(String::as_str),
            Some("/home/probeuserX/y")
        );
    }
}

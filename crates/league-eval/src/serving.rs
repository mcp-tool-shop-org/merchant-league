//! Serving-stack manifest validation, startup Andon gates, wording digests, and environment
//! snapshot capture for reproducible Evaluation Mode runs.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use league_relay::{
    ACTIVE_WORDING_ARM, CONTROL_ARM_WORDING, CONTROL_WORDING_ARM, MODEL_FACING_WORDING,
    PREVIEW_ARM_WORDING_R2, PREVIEW_WORDING_ARM_R2, WordingEntry, sha256_hex,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::EvalError;
use crate::wording::{EVAL_REPLY_WORDING, EVAL_REPLY_WORDING_ARM};

const HOME_PLACEHOLDER: &str = "<home>";
const SERVING_ENV_PREFIXES: [&str; 4] = ["LLAMA_", "CUDA_", "GGML_", "VLLM_"];
const SERVING_ENV_SECRET_SUBSTRINGS: [&str; 5] = ["KEY", "TOKEN", "SECRET", "PASS", "AUTH"];

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

fn is_valid_hex64(raw: &str) -> bool {
    raw.len() == 64 && raw.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn normalize_hex64(raw: &str) -> Option<String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    is_valid_hex64(&trimmed).then_some(trimmed)
}

/// Validate a [`ServingManifestInput`] against all startup reproducibility Andon rules and produce a
/// sanitized [`ServingManifestRecord`] (hashing raw `gpu_uuid` and never retaining it).
pub fn validate_serving_manifest_input(
    input: &ServingManifestInput,
) -> Result<ServingManifestRecord, EvalError> {
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
        EvalError::MissingModelSha256(format!(
            "`model.sha256` must be a non-empty 64-hex SHA-256 digest, got '{}'",
            input.model.sha256
        ))
    })?;
    let chat_template_sha256 = match &input.model.chat_template_sha256 {
        None => None,
        Some(raw) => Some(normalize_hex64(raw).ok_or_else(|| {
            EvalError::MissingServingManifest(format!(
                "`model.chat_template_sha256` must be 64 lowercase hex characters when set, got '{raw}'"
            ))
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
            EvalError::InvalidGpuUuidSha256(format!(
                "`machine.gpu_uuid_sha256` must be 64 hex characters, got '{prehashed}'"
            ))
        })?,
        (Some(raw_uuid), Some(prehashed)) => {
            let expected = sha256_hex(raw_uuid.as_bytes());
            let supplied = normalize_hex64(prehashed).ok_or_else(|| {
                EvalError::InvalidGpuUuidSha256(format!(
                    "`machine.gpu_uuid_sha256` must be 64 hex characters, got '{prehashed}'"
                ))
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

    Ok(ServingManifestRecord {
        engine: ProvenanceTagged {
            value: input.engine,
            provenance: input.server_info_provenance,
        },
        server_version: ProvenanceTagged {
            value: input.server_version.clone(),
            provenance: input.server_info_provenance,
        },
        server_build_commit: input
            .server_build_commit
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|c| ProvenanceTagged {
                value: c.clone(),
                provenance: input.server_info_provenance,
            }),
        determinism_status,
        model: ModelManifestRecord {
            filename: input.model.filename.clone(),
            quant: input.model.quant.clone(),
            sha256: model_sha256,
            chat_template_sha256,
            provenance: input.model.provenance,
        },
        machine: MachineManifestRecord {
            gpu_name: input.machine.gpu_name.clone(),
            gpu_uuid_sha256,
            sm_count: input.machine.sm_count,
            gpu_driver: input.machine.gpu_driver.clone(),
            cuda_runtime_version: input.machine.cuda_runtime_version.clone(),
            rocm_version: input.machine.rocm_version.clone(),
            vram_mib: input.machine.vram_mib,
            os_build: input.machine.os_build.clone(),
            wsl_kernel_version: input.machine.wsl_kernel_version.clone(),
            provenance: input.machine.provenance,
        },
        server_settings: input.server_settings.clone(),
        cache_settings: cache_record,
        sampling: input.sampling.clone(),
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
            "invalid serving manifest JSON {}: {e}",
            path.display()
        ))
    })?;
    validate_serving_manifest_input(&parsed)
}

/// Capture serving-related OS environment variables (`LLAMA_*`, `CUDA_*`, `GGML_*`, `VLLM_*`),
/// omitting secret-named variables and replacing user home directory paths with `<home>`.
#[must_use]
pub fn capture_serving_env_snapshot() -> BTreeMap<String, String> {
    let mut homes = Vec::new();
    for var in ["HOME", "USERPROFILE"] {
        if let Some(val) = std::env::var_os(var)
            && !val.is_empty()
        {
            homes.push(PathBuf::from(val));
        }
    }
    capture_serving_env_snapshot_from(std::env::vars(), &homes)
}

/// Pure testable core of [`capture_serving_env_snapshot`] over explicit env vars and home roots.
#[must_use]
pub fn capture_serving_env_snapshot_from<I>(vars: I, homes: &[PathBuf]) -> BTreeMap<String, String>
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
        out.insert(name, replace_home_paths_in_value(&value, homes));
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
                    .is_none_or(|c| matches!(c, ' ' | '\t' | '=' | ':' | ';' | '"' | '\''));
            let next_ok = i + p_len == haystack.len()
                || haystack[i + p_len..]
                    .chars()
                    .next()
                    .is_some_and(|c| matches!(c, '/' | '\\' | ' ' | '\t' | ':' | ';' | '"' | '\''));
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

        let snap = capture_serving_env_snapshot_from(vars, &homes);
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
            snap.get("CUDA_VISIBLE_DEVICES").map(String::as_str),
            Some("0")
        );
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
}

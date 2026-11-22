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
    /// Recorded thinking start tag emitted by the active chat template (`b11433`
    /// `common/sampling.cpp:315`, `common/chat.cpp:1357-1364`; operator data, not proof the
    /// server binds the reasoning budget).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_start_tag: Option<String>,
    /// Recorded thinking end tags emitted by the active chat template (`b11433`
    /// `tools/server/server-common.cpp:1461`, `common/chat.cpp:1357-1364`, `common/sampling.cpp:315`;
    /// operator data, not proof the server binds the reasoning budget).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_end_tags: Option<Vec<String>>,
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
    /// Recorded thinking start tag reported for the active template (operator data; not proof the
    /// server binds the reasoning budget).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_start_tag: Option<String>,
    /// Recorded thinking end tags reported for the active template (operator data; not proof the
    /// server binds the reasoning budget).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_end_tags: Option<Vec<String>>,
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
    /// Whether Jinja chat templating is enabled (`true` required for reasoning budget enforcement,
    /// `llama.cpp` `b11433` `tools/server/server-context.cpp:1596`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jinja: Option<bool>,
    /// Server reasoning mode flag (`--reasoning`, for example `"on"`, `"off"`, or `"auto"`;
    /// `llama.cpp` `b11433` `tools/server/server-context.cpp:1594-1597`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_mode: Option<String>,
    /// Server default reasoning budget (`--reasoning-budget`, `llama.cpp` `b11433`
    /// `tools/server/server-common.cpp:1458`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reasoning_budget: Option<i32>,
    /// Server reasoning-budget injected message (`--reasoning-budget-message`, `llama.cpp` `b11433`
    /// `tools/server/server-common.cpp:1465`; Evaluation Mode refuses any non-empty setting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_budget_message: Option<String>,
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
/// Verify compiled `built_sha` is valid 40-hex (never `"unknown"`), matches `live_head` inside
/// a git checkout (`INVALID_CRATE_GIT_SHA`), and optionally verifies a clean working tree
/// (`DIRTY_GIT_WORKTREE`).
pub fn validate_crate_git_sha_and_cleanliness(
    built_sha: &str,
    live_head: Option<&str>,
    is_dirty: Option<bool>,
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
    if is_dirty == Some(true) {
        return Err(EvalError::DirtyGitWorktree(
            "git working tree has uncommitted changes; commit cleanly and rebuild before running Evaluation Mode"
                .to_owned(),
        ));
    }
    Ok(normalized)
}

/// Verify compiled `built_sha` is valid 40-hex (never `"unknown"`) and matches `live_head` when
/// running inside a git checkout (`INVALID_CRATE_GIT_SHA`).
pub fn validate_crate_git_sha_against(
    built_sha: &str,
    live_head: Option<&str>,
) -> Result<String, EvalError> {
    validate_crate_git_sha_and_cleanliness(built_sha, live_head, None)
}

fn has_git_ancestor(dir: &Path) -> bool {
    dir.ancestors().any(|anc| anc.join(".git").exists())
}

/// Verify `built_sha` against the git checkout rooted at or above `dir`, refusing dirty working
/// trees (`DIRTY_GIT_WORKTREE`) as well as missing or stale commit SHAs (`INVALID_CRATE_GIT_SHA`).
pub fn resolve_verified_crate_git_sha_in(
    dir: Option<&Path>,
    built_sha: &str,
) -> Result<String, EvalError> {
    let (live_head, is_dirty) = if let Some(dir) = dir
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
        let status_out = Command::new("git")
            .current_dir(dir)
            .args(["status", "--porcelain"])
            .output()
            .map_err(|e| {
                EvalError::InvalidCrateGitSha(format!("unable to inspect git working tree: {e}"))
            })?;
        if !status_out.status.success() {
            return Err(EvalError::InvalidCrateGitSha(
                "git `status --porcelain` failed while verifying clean working tree".to_owned(),
            ));
        }
        let status_str = String::from_utf8(status_out.stdout).map_err(|_| {
            EvalError::InvalidCrateGitSha(
                "non-UTF-8 output from git `status --porcelain`".to_owned(),
            )
        })?;
        (Some(head_str), Some(!status_str.trim().is_empty()))
    } else {
        (None, None)
    };
    validate_crate_git_sha_and_cleanliness(built_sha, live_head.as_deref(), is_dirty)
}

/// Verify compiled [`CRATE_GIT_SHA`] is valid 40-hex (never `"unknown"`), matches the source
/// checkout's current git `HEAD`, and sees a clean git working tree.
pub fn resolve_verified_crate_git_sha() -> Result<String, EvalError> {
    resolve_verified_crate_git_sha_in(
        option_env!("CARGO_MANIFEST_DIR").map(Path::new),
        CRATE_GIT_SHA,
    )
}

/// Source used to resolve and verify the compiled crate git SHA (`LEAGUE_EVAL_GIT_SHA`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrateGitShaSource<'a> {
    /// Inspect the live checkout at compile-time `CARGO_MANIFEST_DIR` against [`CRATE_GIT_SHA`].
    Live,
    /// Inspect an explicit checkout directory against an explicit compiled SHA via the live git path.
    LiveDir {
        /// Checkout directory containing `.git`.
        manifest_dir: &'a Path,
        /// Compiled git commit SHA to validate against `manifest_dir`.
        built_sha: &'a str,
    },
    /// Test seam injecting `built_sha`, optional `live_head`, and optional `is_dirty`.
    Injected {
        /// Compiled git commit SHA to validate.
        built_sha: &'a str,
        /// Optional live `HEAD` commit SHA to compare against.
        live_head: Option<&'a str>,
        /// Optional cleanliness flag (`Some(false)` clean, `Some(true)` dirty).
        is_dirty: Option<bool>,
    },
}

/// Resolve and verify the compiled crate git SHA from [`CrateGitShaSource`].
pub fn resolve_crate_git_sha_from(source: &CrateGitShaSource<'_>) -> Result<String, EvalError> {
    match source {
        CrateGitShaSource::Live => resolve_verified_crate_git_sha(),
        CrateGitShaSource::LiveDir {
            manifest_dir,
            built_sha,
        } => resolve_verified_crate_git_sha_in(Some(manifest_dir), built_sha),
        CrateGitShaSource::Injected {
            built_sha,
            live_head,
            is_dirty,
        } => validate_crate_git_sha_and_cleanliness(built_sha, *live_head, *is_dirty),
    }
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
    clean_server_settings.reasoning_mode = input
        .server_settings
        .reasoning_mode
        .as_deref()
        .map(|s| scrub_manifest_field("server_settings.reasoning_mode", s, &homes))
        .transpose()?;
    clean_server_settings.reasoning_budget_message = input
        .server_settings
        .reasoning_budget_message
        .as_deref()
        .map(|s| scrub_manifest_field("server_settings.reasoning_budget_message", s, &homes))
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
            thinking_start_tag: input
                .model
                .thinking_start_tag
                .as_deref()
                .map(|t| scrub_manifest_field("model.thinking_start_tag", t, &homes))
                .transpose()?,
            thinking_end_tags: input
                .model
                .thinking_end_tags
                .as_ref()
                .map(|tags| {
                    tags.iter()
                        .enumerate()
                        .map(|(i, t)| {
                            scrub_manifest_field(
                                &format!("model.thinking_end_tags[{i}]"),
                                t,
                                &homes,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?,
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

/// Match documented provider API/PAT token shapes with their minimum length floors so
/// benign directory or model path segments such as `/data/hf_models` or `hf_hub` are
/// never redacted or refused:
/// - GitHub fine-grained PAT (`github_pat_` + >=22 alnum + `_` + >=20 alnum)
/// - GitHub classic PAT (`ghp_` + >=36 alnum)
/// - Hugging Face user access token (`hf_` + >=30 alnum)
/// - OpenRouter API key (`sk-or-v1-` + >=64 hex digits)
/// - OpenAI/Stripe scoped secret key (`sk-(proj|svcacct|admin|live|test)-` + >=16 alnum/`_`/`-`)
/// - Legacy OpenAI secret key (`sk-` + >=20 alnum)
fn match_provider_token_len(raw: &[u8], lower: &[u8], pos: usize) -> Option<usize> {
    let prev_ok = pos == 0 || !raw[pos - 1].is_ascii_alphanumeric();
    if !prev_ok {
        return None;
    }
    if lower[pos..].starts_with(b"github_pat_") {
        let mut mid = pos + 11;
        while mid < raw.len() && raw[mid].is_ascii_alphanumeric() {
            mid += 1;
        }
        if mid - (pos + 11) >= 22 && raw.get(mid) == Some(&b'_') {
            let mut end = mid + 1;
            while end < raw.len() && raw[end].is_ascii_alphanumeric() {
                end += 1;
            }
            let trailing_ok = raw
                .get(end)
                .is_none_or(|b| !b.is_ascii_alphanumeric() && *b != b'_');
            if end - (mid + 1) >= 20 && trailing_ok {
                return Some(end - pos);
            }
        }
        return None;
    }
    if lower[pos..].starts_with(b"ghp_") {
        let mut end = pos + 4;
        while end < raw.len() && raw[end].is_ascii_alphanumeric() {
            end += 1;
        }
        let trailing_ok = raw
            .get(end)
            .is_none_or(|b| !b.is_ascii_alphanumeric() && *b != b'_');
        return (end - (pos + 4) >= 36 && trailing_ok).then_some(end - pos);
    }
    if lower[pos..].starts_with(b"hf_") {
        let mut end = pos + 3;
        while end < raw.len() && raw[end].is_ascii_alphanumeric() {
            end += 1;
        }
        let trailing_ok = raw
            .get(end)
            .is_none_or(|b| !b.is_ascii_alphanumeric() && *b != b'_');
        return (end - (pos + 3) >= 30 && trailing_ok).then_some(end - pos);
    }
    if lower[pos..].starts_with(b"sk-or-v1-") {
        let mut end = pos + 9;
        while end < raw.len() && raw[end].is_ascii_hexdigit() {
            end += 1;
        }
        let trailing_ok = raw
            .get(end)
            .is_none_or(|b| !b.is_ascii_alphanumeric() && !matches!(*b, b'_' | b'-'));
        return (end - (pos + 9) >= 64 && trailing_ok).then_some(end - pos);
    }
    for prefix in [
        b"sk-proj-".as_slice(),
        b"sk-svcacct-".as_slice(),
        b"sk-admin-".as_slice(),
        b"sk-live-".as_slice(),
        b"sk-test-".as_slice(),
    ] {
        if lower[pos..].starts_with(prefix) {
            let mut end = pos + prefix.len();
            while end < raw.len()
                && (raw[end].is_ascii_alphanumeric() || matches!(raw[end], b'-' | b'_'))
            {
                end += 1;
            }
            return (end - (pos + prefix.len()) >= 16).then_some(end - pos);
        }
    }
    if lower[pos..].starts_with(b"sk-") {
        let mut end = pos + 3;
        while end < raw.len() && raw[end].is_ascii_alphanumeric() {
            end += 1;
        }
        let trailing_ok = raw
            .get(end)
            .is_none_or(|b| !b.is_ascii_alphanumeric() && !matches!(*b, b'_' | b'-'));
        return (end - (pos + 3) >= 20 && trailing_ok).then_some(end - pos);
    }
    None
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
        if let Some(tok_len) = match_provider_token_len(bytes, lb, i) {
            out.push_str(REDACTED_TOKEN);
            i += tok_len;
            continue;
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
        if match_provider_token_len(bytes, lb, i).is_some() {
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
    fn gpu_and_mig_ids_hashed_never_verbatim() {
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
    fn git_sha_rejects_unknown_and_stale_and_resolves_worktree() {
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

        assert_eq!(CRATE_GIT_SHA.len(), 40);
        assert!(CRATE_GIT_SHA.bytes().all(|b| b.is_ascii_hexdigit()));
        let injected_clean = resolve_crate_git_sha_from(&CrateGitShaSource::Injected {
            built_sha: CRATE_GIT_SHA,
            live_head: Some(CRATE_GIT_SHA),
            is_dirty: Some(false),
        })
        .unwrap_or_else(|e| panic!("injected clean CRATE_GIT_SHA should pass: {e}"));
        assert_eq!(injected_clean, CRATE_GIT_SHA);
        let dirty_err = match resolve_crate_git_sha_from(&CrateGitShaSource::Injected {
            built_sha: CRATE_GIT_SHA,
            live_head: Some(CRATE_GIT_SHA),
            is_dirty: Some(true),
        }) {
            Err(e) => e,
            Ok(v) => panic!("expected DIRTY_GIT_WORKTREE, got {v}"),
        };
        assert_eq!(dirty_err.code(), "DIRTY_GIT_WORKTREE");

        // Verify resolve_verified_crate_git_sha_in against an isolated clean git checkout, then
        // with an untracked file to prove DIRTY_GIT_WORKTREE detection works.
        let repo_tmp = std::env::temp_dir().join(format!(
            "ml_git_sha_unit_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let _ = fs::remove_dir_all(&repo_tmp);
        fs::create_dir_all(&repo_tmp).unwrap_or_else(|e| panic!("mkdir: {e}"));
        let run_git = |args: &[&str]| {
            let out = Command::new("git")
                .current_dir(&repo_tmp)
                .args(args)
                .output()
                .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
            assert!(
                out.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        };
        run_git(&["init", "-q"]);
        run_git(&["config", "user.name", "Eval Test"]);
        run_git(&["config", "user.email", "eval-test@example.invalid"]);
        fs::write(repo_tmp.join("tracked.txt"), "clean\n")
            .unwrap_or_else(|e| panic!("write tracked: {e}"));
        run_git(&["add", "tracked.txt"]);
        run_git(&["commit", "-q", "-m", "init"]);
        let temp_head = run_git(&["rev-parse", "HEAD"]);
        let resolved = resolve_verified_crate_git_sha_in(Some(&repo_tmp), &temp_head)
            .unwrap_or_else(|e| panic!("clean temp repo must resolve: {e}"));
        assert_eq!(resolved, temp_head);
        fs::write(repo_tmp.join("untracked.txt"), "dirty\n")
            .unwrap_or_else(|e| panic!("write untracked: {e}"));
        let temp_dirty = resolve_verified_crate_git_sha_in(Some(&repo_tmp), &temp_head)
            .err()
            .unwrap_or_else(|| panic!("untracked file must trigger DIRTY_GIT_WORKTREE"));
        assert_eq!(temp_dirty.code(), "DIRTY_GIT_WORKTREE");
        let _ = fs::remove_dir_all(&repo_tmp);
    }

    #[test]
    fn home_boundaries_comma_paren_bracket_and_wsl_keep_probeuserx_untouched() {
        let homes = vec![
            PathBuf::from("/home/probeuser"),
            PathBuf::from(r"X:\Profiles\probeuser"),
        ];
        let wsl_home_path = ["", "mnt", "x", "Users", "probeuser", "models", "q.gguf"].join("/");
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
            ("LLAMA_WSL_HOME".to_owned(), wsl_home_path),
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

    #[test]
    fn hf_models_and_hf_hub_paths_pass_through_while_real_provider_tokens_redact() {
        let hf_token = ["hf", "_", "a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6q7r8"].concat();
        let ghp_token = ["ghp", "_", "0123456789abcdefghijklmnopqrstuvwxyz"].concat();
        let gh_pat = [
            "github",
            "_",
            "pat",
            "_",
            "11A2B3C4D5E6F7G8H9I0J1",
            "_",
            "KLMNOPQRSTUVWXYZ0123456789abcdefgh",
        ]
        .concat();
        let sk_proj = ["sk", "-", "proj", "-", "AbCdEfGhIjKlMnOpQrSt"].concat();
        let sk_or = ["sk", "-or-v1-", &"a".repeat(64)].concat();
        let sk_legacy = ["sk", "-", &"Z9y8X7w6V5u4T3s2R1q0".repeat(2)].concat();

        let safe_model_path = "/data/hf_models/x.gguf";
        let safe_hub_path = "/var/cache/hf_hub/weights";
        let scrubbed_model = redact_token_patterns(safe_model_path);
        assert_eq!(scrubbed_model, safe_model_path);
        assert!(!has_residual_unsafe_value(&scrubbed_model, false));
        let scrubbed_hub = redact_token_patterns(safe_hub_path);
        assert_eq!(scrubbed_hub, safe_hub_path);
        assert!(!has_residual_unsafe_value(&scrubbed_hub, false));

        for real_tok in [hf_token, ghp_token, gh_pat, sk_proj, sk_or, sk_legacy] {
            let line = format!("value={real_tok} done");
            assert!(
                has_residual_unsafe_value(&line, false),
                "expected residual check to flag synthetic token"
            );
            let redacted = redact_token_patterns(&line);
            assert!(redacted.contains(REDACTED_TOKEN));
            assert!(!redacted.contains(&real_tok));
            assert!(!has_residual_unsafe_value(&redacted, false));
        }

        let parsed_input = ServingManifestInput {
            engine: EngineKind::LlamaServer,
            server_version: "b11433".to_owned(),
            server_build_commit: Some("0123456789abcdef".to_owned()),
            server_info_provenance: MetadataProvenance::ServerReported,
            model: ModelManifestInput {
                filename: safe_model_path.to_owned(),
                quant: Some("Q4_K_M".to_owned()),
                sha256: "a".repeat(64),
                chat_template_sha256: Some("b".repeat(64)),
                thinking_start_tag: Some("<think>".to_owned()),
                thinking_end_tags: Some(vec!["</think>".to_owned()]),
                provenance: MetadataProvenance::OperatorSupplied,
            },
            machine: MachineManifestInput {
                gpu_name: "NVIDIA H100".to_owned(),
                gpu_uuid: Some("gpu-test-uuid-0".to_owned()),
                gpu_uuid_sha256: None,
                sm_count: None,
                gpu_driver: "570.12".to_owned(),
                cuda_runtime_version: None,
                rocm_version: None,
                vram_mib: None,
                os_build: safe_hub_path.to_owned(),
                wsl_kernel_version: None,
                provenance: MetadataProvenance::OperatorSupplied,
            },
            server_settings: ServerSettingsRecord {
                parallel_slots: 1,
                batch_size: 2048,
                ubatch_size: 512,
                kv_unified: Some(false),
                flash_attn: Some(true),
                cache_type_k: "f16".to_owned(),
                cache_type_v: "f16".to_owned(),
                speculative_decoding: Some("none".to_owned()),
                ctx_size: 32768,
                ctx_shift: false,
                threads: Some(16),
                jinja: Some(true),
                reasoning_mode: Some("on".to_owned()),
                default_reasoning_budget: Some(-1),
                reasoning_budget_message: None,
                provenance: MetadataProvenance::ServerReported,
            },
            cache_settings: CacheSettingsInput {
                per_request_cache_prompt: Some(false),
                no_cache_prompt_flag: true,
                cache_ram_mib: Some(0),
                cache_idle_slots: Some(false),
                enable_prefix_caching: None,
                provenance: MetadataProvenance::ServerReported,
            },
            sampling: EffectiveSamplingRecord {
                temperature: 0.0,
                top_k: 0,
                top_p: 1.0,
                min_p: 0.0,
                repeat_penalty: 1.0,
                presence_penalty: 0.0,
                frequency_penalty: 0.0,
                seed: Some(7),
                stop_sequences: Vec::new(),
                max_tokens: 512,
                sampler_order: Vec::new(),
                provenance: MetadataProvenance::OperatorSupplied,
            },
        };
        let rec = validate_serving_manifest_input(&parsed_input)
            .unwrap_or_else(|e| panic!("hf_models/hf_hub paths must validate cleanly: {e}"));
        assert_eq!(rec.model.filename, safe_model_path);
        assert_eq!(rec.machine.os_build, safe_hub_path);
    }

    #[test]
    fn dirty_git_worktree_refuses_with_named_code() {
        let head = "a".repeat(40);
        let dirty_err = match validate_crate_git_sha_and_cleanliness(&head, Some(&head), Some(true))
        {
            Err(e) => e,
            Ok(v) => panic!("dirty worktree flag must refuse, got {v}"),
        };
        assert_eq!(dirty_err.code(), "DIRTY_GIT_WORKTREE");

        let clean_sha = validate_crate_git_sha_and_cleanliness(&head, Some(&head), Some(false))
            .unwrap_or_else(|e| panic!("clean flag should pass: {e}"));
        assert_eq!(clean_sha, head);

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let repo =
            std::env::temp_dir().join(format!("ml_dirty_git_test_{}_{stamp}", std::process::id()));
        let _ = fs::remove_dir_all(&repo);
        fs::create_dir_all(&repo).unwrap_or_else(|e| panic!("mkdir repo: {e}"));
        let run_git = |args: &[&str]| {
            let out = Command::new("git")
                .current_dir(&repo)
                .args(args)
                .output()
                .unwrap_or_else(|e| panic!("git {:?}: {e}", args));
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        };
        run_git(&["init", "-b", "main"]);
        run_git(&["config", "user.email", "test@example.com"]);
        run_git(&["config", "user.name", "Test"]);
        fs::write(repo.join("tracked.txt"), "v1\n").unwrap_or_else(|e| panic!("write: {e}"));
        run_git(&["add", "tracked.txt"]);
        run_git(&["commit", "-m", "init"]);
        let head_sha = run_git(&["rev-parse", "HEAD"]);

        let ok_sha = resolve_verified_crate_git_sha_in(Some(&repo), &head_sha)
            .unwrap_or_else(|e| panic!("clean repo should pass: {e}"));
        assert_eq!(ok_sha, head_sha);

        fs::write(repo.join("tracked.txt"), "dirty\n").unwrap_or_else(|e| panic!("dirty: {e}"));
        let real_dirty_err = match resolve_verified_crate_git_sha_in(Some(&repo), &head_sha) {
            Err(e) => e,
            Ok(v) => panic!("modified tracked file must trigger DIRTY_GIT_WORKTREE, got {v}"),
        };
        assert_eq!(real_dirty_err.code(), "DIRTY_GIT_WORKTREE");
        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn build_rs_watches_worktree_common_dir_branch_ref_and_tracks_commits() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let root = std::env::temp_dir().join(format!(
            "ml_build_rs_worktree_test_{}_{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let main_repo = root.join("main_repo");
        let wt_repo = root.join("wt_repo");
        fs::create_dir_all(&main_repo).unwrap_or_else(|e| panic!("mkdir: {e}"));

        let run_git = |cwd: &Path, args: &[&str]| {
            let out = Command::new("git")
                .current_dir(cwd)
                .args(args)
                .output()
                .unwrap_or_else(|e| panic!("git {:?}: {e}", args));
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        };
        run_git(&main_repo, &["init", "-b", "main"]);
        run_git(&main_repo, &["config", "user.email", "test@example.com"]);
        run_git(&main_repo, &["config", "user.name", "Test"]);
        fs::write(main_repo.join("f.txt"), "c1\n").unwrap_or_else(|e| panic!("write c1: {e}"));
        run_git(&main_repo, &["add", "f.txt"]);
        run_git(&main_repo, &["commit", "-m", "commit 1"]);
        run_git(
            &main_repo,
            &[
                "worktree",
                "add",
                "-b",
                "wt-branch",
                &wt_repo.to_string_lossy(),
            ],
        );
        let wt_head_1 = run_git(&wt_repo, &["rev-parse", "HEAD"]);

        let build_rs = Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs");
        let exe_name = if cfg!(windows) {
            "eval_build_probe.exe"
        } else {
            "eval_build_probe"
        };
        let build_bin = root.join(exe_name);
        let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
        let compile_out = Command::new(&rustc)
            .arg("--edition")
            .arg("2024")
            .arg(&build_rs)
            .arg("-o")
            .arg(&build_bin)
            .output()
            .unwrap_or_else(|e| panic!("compile build.rs with {rustc}: {e}"));
        assert!(
            compile_out.status.success(),
            "rustc build.rs failed: {}",
            String::from_utf8_lossy(&compile_out.stderr)
        );

        let run_probe = |manifest_dir: &Path| {
            let out = Command::new(&build_bin)
                .env("CARGO_MANIFEST_DIR", manifest_dir)
                .output()
                .unwrap_or_else(|e| panic!("run build.rs probe: {e}"));
            assert!(out.status.success());
            String::from_utf8_lossy(&out.stdout).into_owned()
        };

        let out_wt_1 = run_probe(&wt_repo);
        assert!(out_wt_1.contains(&format!("cargo:rustc-env=LEAGUE_EVAL_GIT_SHA={wt_head_1}")));
        let watches_1: Vec<PathBuf> = out_wt_1
            .lines()
            .filter_map(|l| l.strip_prefix("cargo:rerun-if-changed="))
            .map(PathBuf::from)
            .collect();
        let common_branch_ref = fs::canonicalize(main_repo.join(".git/refs/heads/wt-branch"))
            .unwrap_or_else(|e| panic!("canonicalize common branch ref: {e}"));
        assert!(
            watches_1.iter().any(|p| {
                fs::canonicalize(p)
                    .map(|c| c == common_branch_ref)
                    .unwrap_or(false)
            }),
            "build.rs must watch common-dir branch ref {} in worktree, got {watches_1:?}",
            common_branch_ref.display()
        );

        let before_ref_bytes =
            fs::read(&common_branch_ref).unwrap_or_else(|e| panic!("read ref 1: {e}"));
        let head_file_before =
            fs::read_to_string(run_git(&wt_repo, &["rev-parse", "--git-dir"]) + "/HEAD")
                .unwrap_or_default();

        fs::write(wt_repo.join("f.txt"), "c2\n").unwrap_or_else(|e| panic!("write c2: {e}"));
        run_git(&wt_repo, &["add", "f.txt"]);
        run_git(&wt_repo, &["commit", "-m", "commit 2"]);
        let wt_head_2 = run_git(&wt_repo, &["rev-parse", "HEAD"]);
        assert_ne!(wt_head_1, wt_head_2);

        let after_ref_bytes =
            fs::read(&common_branch_ref).unwrap_or_else(|e| panic!("read ref 2: {e}"));
        let head_file_after =
            fs::read_to_string(run_git(&wt_repo, &["rev-parse", "--git-dir"]) + "/HEAD")
                .unwrap_or_default();
        assert_eq!(head_file_before, head_file_after);
        assert_ne!(
            before_ref_bytes, after_ref_bytes,
            "HEAD file stays unchanged across branch commits while watched common branch ref updates"
        );

        let out_wt_2 = run_probe(&wt_repo);
        assert!(out_wt_2.contains(&format!("cargo:rustc-env=LEAGUE_EVAL_GIT_SHA={wt_head_2}")));

        let _ = fs::remove_dir_all(&root);
    }
}

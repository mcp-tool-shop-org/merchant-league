//! Seat configuration file parsing and validation.
//!
//! A seat file is JSON (either `{"seats": [...]}` or a top-level `[...]`),
//! listing each seated guild, its backend (`local`, `openrouter`, or `file`),
//! per-seat sampling, wall timeout, per-call cost estimate (`max_gpu_secs_per_call`
//! in GPU-seconds for `local`, `max_cost_per_call` in credits for `openrouter`),
//! and optional pinned `reasoning` settings (`B4.3`).

use crate::error::RelayError;
use crate::secret::{ApiKey, redact_secrets};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

/// Minimum safety margin in seconds required between a `local` seat's `timeout_secs`
/// and its reserved `max_gpu_secs_per_call` (`timeout_secs + LOCAL_TIMEOUT_MARGIN_SECS <= max_gpu_secs_per_call`).
///
/// Covers thread-channel wakeup after `recv_timeout` while post-call accounting also
/// clamps `charged_credits` to at most `max_gpu_secs_per_call`.
pub const LOCAL_TIMEOUT_MARGIN_SECS: f64 = 0.05;

/// Unit in which a turn's ledger reserves and charges per-call spend.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostUnit {
    /// OpenRouter account credits (`--cap-credits`).
    #[default]
    Credits,
    /// Client-measured wall-clock seconds (`--cap-gpu-secs`).
    ///
    /// Note (`B-7`): this measures client wall-clock time around each HTTP attempt, not
    /// hardware profiler counters. When multiple seats run concurrently against one local
    /// server, their wall times sum to at least the elapsed server GPU time (over-counting,
    /// fail-safe toward the cap). Conversely, if a client times out or disconnects and the
    /// server continues generating tokens after socket close, that server-side tail is not
    /// visible in client wall-clock time (under-counting).
    GpuSeconds,
}

impl CostUnit {
    /// Wire and display name (`"credits"` or `"gpu_seconds"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Credits => "credits",
            Self::GpuSeconds => "gpu_seconds",
        }
    }
}

impl fmt::Display for CostUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Seat backend selector in the seat configuration file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    /// Automated chat-completions seat called via an OpenAI-compatible local VM server (e.g. vLLM).
    Local,
    /// Automated chat-completions seat called via OpenRouter.
    Openrouter,
    /// Manual seat relayed by hand (for example Claude Code subagents).
    File,
}

impl BackendKind {
    /// Lowercase backend label (`"local"`, `"openrouter"`, or `"file"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Openrouter => "openrouter",
            Self::File => "file",
        }
    }
}

/// Pinned reasoning effort level supported by vLLM (`reasoning_effort`) and OpenRouter (`reasoning.effort`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    /// Disable reasoning (`"none"`).
    None,
    /// Minimal reasoning (`"minimal"`).
    Minimal,
    /// Low reasoning (`"low"`).
    Low,
    /// Medium reasoning (`"medium"`).
    Medium,
    /// High reasoning (`"high"`).
    High,
    /// Extra-high reasoning (`"xhigh"`).
    Xhigh,
    /// Maximum reasoning (`"max"`).
    Max,
}

impl ReasoningEffort {
    /// Wire string (`"none"`..`"max"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

/// Pinned per-seat reasoning configuration in `seats.json` (`B4.3`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatReasoningConfig {
    /// Optional reasoning effort (`reasoning_effort` on local vLLM; `reasoning.effort` on OpenRouter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffort>,
    /// Optional reasoning token cap (`thinking_token_budget` on local vLLM; `reasoning.max_tokens` on OpenRouter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
    /// Optional model-card chat-template flags (for example `{"enable_thinking": false}` or `{"thinking": true}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_template_kwargs: Option<BTreeMap<String, serde_json::Value>>,
    /// Whether intermediate thinking is excluded from persisted transcripts and `from-seat/` replies.
    pub exclude_from_transcript: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeatFileWrappedWire {
    seats: Vec<SeatEntryWire>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeatEntryWire {
    guild: String,
    backend: BackendKind,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    timeout_secs: Option<f64>,
    #[serde(default)]
    max_cost_per_call: Option<f64>,
    #[serde(default)]
    max_gpu_secs_per_call: Option<f64>,
    #[serde(default)]
    reasoning: Option<SeatReasoningConfig>,
}

/// Validated parameters for a `local` OpenAI-compatible seat (such as vLLM on the VM).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalSeat {
    /// Validated configured base URL (loopback or explicitly allowed VM-private IP).
    pub base_url: String,
    /// Normalized `/v1/chat/completions` URL for this seat's server.
    pub endpoint_url: String,
    /// Normalized `/tokenize` URL (`<origin>/tokenize`) for counting scratchpad tokens (`A-3`).
    pub tokenize_url: String,
    /// Normalized host extracted from `base_url`.
    pub host: String,
    /// Pinned local model identifier served at `base_url`.
    pub model: String,
    /// Sampling temperature in `0.0..=2.0`.
    pub temperature: f64,
    /// Upper bound on generated tokens (`max_tokens`).
    pub max_output_tokens: u32,
    /// Per-call wall-clock timeout for this seat (`timeout + LOCAL_TIMEOUT_MARGIN_SECS <= max_gpu_secs_per_call`).
    pub timeout: Duration,
    /// Worst-case single-call cost estimate in client wall-clock GPU-seconds reserved against `--cap-gpu-secs`.
    pub max_gpu_secs_per_call: f64,
    /// Optional pinned reasoning settings (`B4.3`).
    pub reasoning: Option<SeatReasoningConfig>,
}

/// Validated parameters for an `openrouter` seat.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterSeat {
    /// Exact pinned OpenRouter slug (`provider/model-slug`), never a floating alias.
    pub model: String,
    /// Sampling temperature in `0.0..=2.0`.
    pub temperature: f64,
    /// Upper bound on generated tokens (`max_tokens`).
    pub max_output_tokens: u32,
    /// Per-call wall-clock timeout for this seat.
    pub timeout: Duration,
    /// Worst-case single-call cost estimate reserved against `--cap-credits` before calling.
    pub max_cost_per_call: f64,
    /// Optional pinned reasoning settings (`B4.3`).
    pub reasoning: Option<SeatReasoningConfig>,
}

/// Validated backend configuration for one seat.
#[derive(Debug, Clone, PartialEq)]
pub enum SeatBackend {
    /// Relayed automatically through a local OpenAI-compatible server on the VM.
    Local(LocalSeat),
    /// Relayed automatically through OpenRouter (or a mock [`Transport`](crate::transport::Transport)).
    Openrouter(OpenRouterSeat),
    /// Relayed manually outside `league-relay`; reported and skipped.
    File {
        /// Optional model note carried in the seat file for reference.
        model: Option<String>,
    },
}

/// One validated seat entry from the seat file.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatSpec {
    /// Guild identifier used in `to-seat/`, `from-seat/`, and `relay/seats/<Guild>.jsonl`.
    pub guild: String,
    /// Backend configuration.
    pub backend: SeatBackend,
}

/// Validated collection of seats for a game.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatFile {
    /// Ordered list of seats.
    pub seats: Vec<SeatSpec>,
}

fn seat_err(msg: impl AsRef<str>, key: Option<&ApiKey>) -> RelayError {
    RelayError::SeatConfig(redact_secrets(msg.as_ref(), key))
}

impl SeatFile {
    /// Read and validate a JSON seat file from `path` (strict loopback-only for `local` seats).
    pub fn load(path: &Path, redaction_key: Option<&ApiKey>) -> Result<Self, RelayError> {
        Self::load_with_allowed_hosts(path, redaction_key, &[])
    }

    /// Read and validate a JSON seat file from `path`, allowing named VM-local hosts on `local` seats.
    pub fn load_with_allowed_hosts(
        path: &Path,
        redaction_key: Option<&ApiKey>,
        allowed_vm_hosts: &[String],
    ) -> Result<Self, RelayError> {
        let raw = fs::read_to_string(path).map_err(|e| {
            seat_err(
                format!("could not read {}: {e}", path.display()),
                redaction_key,
            )
        })?;
        Self::from_json_str_with_allowed_hosts(&raw, redaction_key, allowed_vm_hosts)
    }

    /// Parse and validate a JSON seat file from `raw` (strict loopback-only for `local` seats).
    pub fn from_json_str(raw: &str, redaction_key: Option<&ApiKey>) -> Result<Self, RelayError> {
        Self::from_json_str_with_allowed_hosts(raw, redaction_key, &[])
    }

    /// Parse and validate a JSON seat file from `raw`, allowing named VM-local hosts on `local` seats.
    pub fn from_json_str_with_allowed_hosts(
        raw: &str,
        redaction_key: Option<&ApiKey>,
        allowed_vm_hosts: &[String],
    ) -> Result<Self, RelayError> {
        let normalized_allowed = normalize_allowed_vm_hosts(allowed_vm_hosts)?;
        let value: serde_json::Value = serde_json::from_str(raw)
            .map_err(|e| seat_err(format!("invalid JSON: {e}"), redaction_key))?;
        if contains_forbidden_key_field(&value) {
            return Err(RelayError::SecretInForbiddenPlace(
                "seat file must not contain an API key or authorization credential".to_owned(),
            ));
        }

        let entries: Vec<SeatEntryWire> = if value.is_array() {
            serde_json::from_value(value).map_err(|e| seat_err(e.to_string(), redaction_key))?
        } else {
            serde_json::from_value::<SeatFileWrappedWire>(value)
                .map_err(|e| seat_err(e.to_string(), redaction_key))?
                .seats
        };

        if entries.is_empty() {
            return Err(RelayError::SeatConfig(
                "seat file must define at least one seat".to_owned(),
            ));
        }

        let mut seen_guilds = HashSet::with_capacity(entries.len());
        let mut seen_local_hosts = HashSet::with_capacity(entries.len());
        let mut seats = Vec::with_capacity(entries.len());
        let mut saw_openrouter = false;
        let mut saw_local = false;

        for entry in entries {
            validate_guild_name(&entry.guild)?;
            if !seen_guilds.insert(entry.guild.to_ascii_lowercase()) {
                return Err(RelayError::SeatConfig(format!(
                    "duplicate guild '{}' (case-insensitive) in seat file",
                    entry.guild
                )));
            }

            let backend = match entry.backend {
                BackendKind::File => {
                    if entry.base_url.is_some()
                        || entry.max_cost_per_call.is_some()
                        || entry.max_gpu_secs_per_call.is_some()
                        || entry.reasoning.is_some()
                    {
                        return Err(RelayError::SeatConfig(format!(
                            "manual 'file' seat '{}' must not set base_url, cost estimates, or reasoning",
                            entry.guild
                        )));
                    }
                    SeatBackend::File { model: entry.model }
                }
                BackendKind::Openrouter => {
                    saw_openrouter = true;
                    if entry.base_url.is_some() {
                        return Err(RelayError::SeatConfig(format!(
                            "openrouter seat '{}' must not set 'base_url'",
                            entry.guild
                        )));
                    }
                    if entry.max_gpu_secs_per_call.is_some() {
                        return Err(RelayError::SeatConfig(format!(
                            "openrouter seat '{}' must set 'max_cost_per_call', not 'max_gpu_secs_per_call'",
                            entry.guild
                        )));
                    }
                    let model = entry.model.ok_or_else(|| {
                        RelayError::SeatConfig(format!("seat '{}' missing 'model'", entry.guild))
                    })?;
                    validate_pinned_openrouter_slug(&entry.guild, &model)?;

                    let (temperature, max_output_tokens, timeout) = validate_common_sampling(
                        &entry.guild,
                        entry.temperature,
                        entry.max_output_tokens,
                        entry.timeout_secs,
                    )?;

                    let Some(max_cost_per_call) = entry
                        .max_cost_per_call
                        .filter(|c| c.is_finite() && *c > 0.0)
                    else {
                        return Err(RelayError::SeatConfig(format!(
                            "seat '{}' requires finite positive 'max_cost_per_call'",
                            entry.guild
                        )));
                    };

                    if let Some(ref r) = entry.reasoning {
                        validate_seat_reasoning(&entry.guild, r)?;
                    }

                    SeatBackend::Openrouter(OpenRouterSeat {
                        model,
                        temperature,
                        max_output_tokens,
                        timeout,
                        max_cost_per_call,
                        reasoning: entry.reasoning,
                    })
                }
                BackendKind::Local => {
                    saw_local = true;
                    if entry.max_cost_per_call.is_some() {
                        return Err(RelayError::SeatConfig(format!(
                            "local seat '{}' must set 'max_gpu_secs_per_call' (GPU-seconds), not 'max_cost_per_call'",
                            entry.guild
                        )));
                    }
                    let raw_base_url = entry.base_url.ok_or_else(|| {
                        RelayError::SeatConfig(format!(
                            "local seat '{}' requires 'base_url'",
                            entry.guild
                        ))
                    })?;
                    let (endpoint_url, tokenize_url, host) =
                        validate_local_endpoints(&entry.guild, &raw_base_url, &normalized_allowed)?;

                    let model = entry.model.ok_or_else(|| {
                        RelayError::SeatConfig(format!("seat '{}' missing 'model'", entry.guild))
                    })?;
                    validate_pinned_local_model(&entry.guild, &model)?;

                    let (temperature, max_output_tokens, timeout) = validate_common_sampling(
                        &entry.guild,
                        entry.temperature,
                        entry.max_output_tokens,
                        entry.timeout_secs,
                    )?;
                    let timeout_secs = entry.timeout_secs.unwrap_or(timeout.as_secs_f64());

                    let Some(max_gpu_secs_per_call) = entry
                        .max_gpu_secs_per_call
                        .filter(|c| c.is_finite() && *c > 0.0)
                    else {
                        return Err(RelayError::SeatConfig(format!(
                            "local seat '{}' requires finite positive 'max_gpu_secs_per_call'",
                            entry.guild
                        )));
                    };

                    if (max_gpu_secs_per_call - timeout_secs) + 1e-9 < LOCAL_TIMEOUT_MARGIN_SECS {
                        return Err(RelayError::SeatConfig(format!(
                            "local seat '{}' requires timeout_secs ({timeout_secs}) + {LOCAL_TIMEOUT_MARGIN_SECS} <= max_gpu_secs_per_call ({max_gpu_secs_per_call}) so a call running to its timeout cannot exceed its GPU-second reservation",
                            entry.guild
                        )));
                    }

                    if let Some(ref r) = entry.reasoning {
                        validate_local_seat_reasoning(&entry.guild, r)?;
                    }

                    seen_local_hosts.insert(host.clone());
                    SeatBackend::Local(LocalSeat {
                        base_url: raw_base_url,
                        endpoint_url,
                        tokenize_url,
                        host,
                        model,
                        temperature,
                        max_output_tokens,
                        timeout,
                        max_gpu_secs_per_call,
                        reasoning: entry.reasoning,
                    })
                }
            };

            if saw_openrouter && saw_local {
                return Err(RelayError::SeatConfig(
                    "seat file cannot mix 'openrouter' (credits) and 'local' (gpu_seconds) seats in one ledger".to_owned(),
                ));
            }

            seats.push(SeatSpec {
                guild: entry.guild,
                backend,
            });
        }

        for allowed_host in &normalized_allowed {
            if !seen_local_hosts.contains(allowed_host) {
                return Err(RelayError::SeatConfig(format!(
                    "--allow-vm-host '{allowed_host}' is not referenced by any 'local' seat in the seat file"
                )));
            }
        }

        Ok(Self { seats })
    }

    /// Whether at least one seat uses the `openrouter` backend.
    pub fn has_openrouter_seat(&self) -> bool {
        self.seats
            .iter()
            .any(|s| matches!(s.backend, SeatBackend::Openrouter(_)))
    }

    /// Whether at least one seat uses the `local` backend.
    pub fn has_local_seat(&self) -> bool {
        self.seats
            .iter()
            .any(|s| matches!(s.backend, SeatBackend::Local(_)))
    }

    /// Active cost unit across automated seats (`None` when all seats are manual `file` seats).
    pub fn cost_unit(&self) -> Option<CostUnit> {
        if self.has_local_seat() {
            Some(CostUnit::GpuSeconds)
        } else if self.has_openrouter_seat() {
            Some(CostUnit::Credits)
        } else {
            None
        }
    }
}

fn validate_common_sampling(
    guild: &str,
    temperature_opt: Option<f64>,
    max_tokens_opt: Option<u32>,
    timeout_opt: Option<f64>,
) -> Result<(f64, u32, Duration), RelayError> {
    let Some(temperature) = temperature_opt.filter(|t| t.is_finite() && (0.0..=2.0).contains(t))
    else {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' requires finite 'temperature' in 0.0..=2.0"
        )));
    };
    let Some(max_output_tokens) = max_tokens_opt.filter(|&m| m > 0) else {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' requires positive 'max_output_tokens'"
        )));
    };
    let Some(timeout_secs) = timeout_opt.filter(|t| t.is_finite() && *t > 0.0) else {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' requires finite positive 'timeout_secs'"
        )));
    };
    Ok((
        temperature,
        max_output_tokens,
        Duration::from_secs_f64(timeout_secs),
    ))
}

fn validate_seat_reasoning(guild: &str, r: &SeatReasoningConfig) -> Result<(), RelayError> {
    let has_kwargs = r
        .chat_template_kwargs
        .as_ref()
        .is_some_and(|m| !m.is_empty());
    if r.effort.is_none() && r.budget_tokens.is_none() && !has_kwargs {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' reasoning block must pin at least one of 'effort', 'budget_tokens', or 'chat_template_kwargs'"
        )));
    }
    if let Some(b) = r.budget_tokens {
        if b == 0 {
            return Err(RelayError::SeatConfig(format!(
                "seat '{guild}' reasoning 'budget_tokens' must be positive"
            )));
        }
        if r.effort == Some(ReasoningEffort::None) {
            return Err(RelayError::SeatConfig(format!(
                "seat '{guild}' reasoning cannot combine effort='none' with 'budget_tokens'"
            )));
        }
    }
    if let Some(kwargs) = &r.chat_template_kwargs {
        for (k, v) in kwargs {
            if k.trim().is_empty() {
                return Err(RelayError::SeatConfig(format!(
                    "seat '{guild}' chat_template_kwargs key must be non-empty"
                )));
            }
            let valid_primitive =
                v.is_boolean() || v.as_f64().is_some_and(f64::is_finite) || v.is_string();
            if !valid_primitive {
                return Err(RelayError::SeatConfig(format!(
                    "seat '{guild}' chat_template_kwargs '{k}' must be a boolean, number, or string"
                )));
            }
            if matches!(k.as_str(), "enable_thinking" | "thinking")
                && v == &serde_json::Value::Bool(false)
                && r.budget_tokens.is_some()
            {
                return Err(RelayError::SeatConfig(format!(
                    "seat '{guild}' reasoning cannot disable '{k}' while setting 'budget_tokens'"
                )));
            }
        }
    }
    Ok(())
}

fn validate_local_seat_reasoning(guild: &str, r: &SeatReasoningConfig) -> Result<(), RelayError> {
    validate_seat_reasoning(guild, r)?;
    if let Some(effort) = r.effort
        && !matches!(
            effort,
            ReasoningEffort::None
                | ReasoningEffort::Low
                | ReasoningEffort::Medium
                | ReasoningEffort::High
        )
    {
        return Err(RelayError::SeatConfig(format!(
            "local seat '{guild}' reasoning effort '{}' is refused: local OpenAI-compatible servers only accept 'none', 'low', 'medium', or 'high' without a custom server-side reasoning-effort-map",
            effort.as_str()
        )));
    }
    Ok(())
}

/// Validate a VM-local host allowance passed via `--allow-vm-host <host>`.
///
/// Permits only `localhost`, loopback (`127.0.0.0/8`, `::1`), RFC1918 IPv4 (`10.0.0.0/8`,
/// `172.16.0.0/12`, `192.168.0.0/16`), and IPv6 ULA (`fc00::/7`) (`B-8`).
/// Refuses unspecified (`0.0.0.0`, `::`), link-local (`169.254.0.0/16` cloud metadata and
/// `fe80::/10`), IPv4-mapped IPv6 (`::ffff:...`), public internet IPs, and all DNS hostnames
/// (both dotted domains and single-label search-domain names such as `metadata` or `vm`).
pub fn validate_vm_host_allowance(raw_host: &str) -> Result<String, RelayError> {
    let trimmed = raw_host.trim();
    let unbracketed = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    if unbracketed.is_empty()
        || unbracketed
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | '@' | '?' | '#'))
    {
        return Err(RelayError::Usage(
            "--allow-vm-host must be 'localhost' or a bare loopback/RFC1918/ULA IP address without scheme, port, or path"
                .to_owned(),
        ));
    }
    if let Ok(ip) = unbracketed.parse::<IpAddr>() {
        if ip.is_unspecified() {
            return Err(RelayError::Usage(
                "--allow-vm-host cannot be the unspecified address (0.0.0.0 or ::)".to_owned(),
            ));
        }
        let is_vm_local = match ip {
            IpAddr::V4(v4) => {
                if v4.is_link_local() {
                    return Err(RelayError::Usage(
                        "--allow-vm-host refuses IPv4 link-local (169.254.0.0/16, including cloud metadata); use loopback or RFC1918"
                            .to_owned(),
                    ));
                }
                v4.is_loopback() || v4.is_private()
            }
            IpAddr::V6(v6) => {
                if v6.to_ipv4_mapped().is_some() || v6.is_unicast_link_local() {
                    return Err(RelayError::Usage(
                        "--allow-vm-host refuses IPv6 link-local (fe80::/10) and IPv4-mapped IPv6 addresses; use ::1 or ULA (fc00::/7)"
                            .to_owned(),
                    ));
                }
                v6.is_loopback() || v6.is_unique_local()
            }
        };
        if !is_vm_local {
            return Err(RelayError::Usage(
                "--allow-vm-host refuses public IP addresses; only loopback, RFC1918, or IPv6 ULA addresses are allowed"
                    .to_owned(),
            ));
        }
        return Ok(ip.to_string());
    }
    if unbracketed.contains(':') {
        return Err(RelayError::Usage(
            "--allow-vm-host must not include a ':port' suffix".to_owned(),
        ));
    }
    let lower = unbracketed.to_ascii_lowercase();
    if lower == "localhost" {
        return Ok(lower);
    }
    Err(RelayError::Usage(
        "--allow-vm-host refuses DNS hostnames (single-label and dotted); pass 'localhost' or an explicit loopback/RFC1918/ULA IP literal"
            .to_owned(),
    ))
}

/// Validate and normalize a slice of `--allow-vm-host` values.
pub fn normalize_allowed_vm_hosts(raw_hosts: &[String]) -> Result<Vec<String>, RelayError> {
    let mut out = Vec::with_capacity(raw_hosts.len());
    for h in raw_hosts {
        let norm = validate_vm_host_allowance(h)?;
        if !out.contains(&norm) {
            out.push(norm);
        }
    }
    Ok(out)
}

fn is_host_loopback_or_allowed(host: &str, normalized_allowed: &[String]) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(v4) => {
                if v4.is_loopback() {
                    return true;
                }
                if v4.is_private() {
                    let canon = ip.to_string();
                    return normalized_allowed.iter().any(|a| a == &canon);
                }
                return false;
            }
            IpAddr::V6(v6) => {
                if v6.to_ipv4_mapped().is_some() || v6.is_unicast_link_local() {
                    return false;
                }
                if v6.is_loopback() {
                    return true;
                }
                if v6.is_unique_local() {
                    let canon = ip.to_string();
                    return normalized_allowed.iter().any(|a| a == &canon);
                }
                return false;
            }
        }
    }
    false
}

/// Validate a `local` seat's `base_url` and normalize it to
/// `(<origin>/v1/chat/completions, <origin>/tokenize, <canonical_host>)`.
pub fn validate_local_endpoints(
    guild: &str,
    base_url: &str,
    normalized_allowed: &[String],
) -> Result<(String, String, String), RelayError> {
    let (origin, canonical_host, clean_path) =
        parse_and_validate_local_origin(guild, base_url, normalized_allowed)?;
    match clean_path {
        "" | "/v1" | "/v1/chat/completions" => {}
        _ => {
            return Err(RelayError::SeatConfig(format!(
                "local seat '{guild}' base_url path must be '/', '/v1', or '/v1/chat/completions'"
            )));
        }
    }
    Ok((
        format!("{origin}/v1/chat/completions"),
        format!("{origin}/tokenize"),
        canonical_host,
    ))
}

/// Validate a `local` seat's `base_url` and normalize it to `<origin>/v1/chat/completions`.
///
/// Refuses credentials in URL, non-HTTP(S) schemes, query/fragment components, and any
/// host that is neither loopback (`127.0.0.0/8`, `[::1]`, `localhost`) nor explicitly
/// named in `normalized_allowed`.
pub fn validate_local_base_url(
    guild: &str,
    base_url: &str,
    normalized_allowed: &[String],
) -> Result<(String, String), RelayError> {
    let (endpoint_url, _tokenize_url, canonical_host) =
        validate_local_endpoints(guild, base_url, normalized_allowed)?;
    Ok((endpoint_url, canonical_host))
}

/// Validate a `local` seat's `/tokenize` URL and normalize it to `<origin>/tokenize`.
pub fn validate_local_tokenize_url(
    guild: &str,
    url: &str,
    normalized_allowed: &[String],
) -> Result<(String, String), RelayError> {
    let (origin, canonical_host, clean_path) =
        parse_and_validate_local_origin(guild, url, normalized_allowed)?;
    match clean_path {
        "" | "/v1" | "/tokenize" => {}
        _ => {
            return Err(RelayError::SeatConfig(format!(
                "local seat '{guild}' tokenize URL path must be '/tokenize'"
            )));
        }
    }
    Ok((format!("{origin}/tokenize"), canonical_host))
}

fn parse_and_validate_local_origin<'a>(
    guild: &str,
    raw_url: &'a str,
    normalized_allowed: &[String],
) -> Result<(String, String, &'a str), RelayError> {
    let trimmed = raw_url.trim();
    let (scheme, after_scheme) = if let Some(rest) = trimmed.strip_prefix("http://") {
        ("http", rest)
    } else if let Some(rest) = trimmed.strip_prefix("https://") {
        ("https", rest)
    } else {
        return Err(RelayError::SeatConfig(format!(
            "local seat '{guild}' base_url must start with http:// or https://"
        )));
    };

    if after_scheme.contains('?') || after_scheme.contains('#') {
        return Err(RelayError::SeatConfig(format!(
            "local seat '{guild}' base_url must not contain query or fragment parameters"
        )));
    }

    let (authority, raw_path) = match after_scheme.find('/') {
        Some(idx) => (&after_scheme[..idx], &after_scheme[idx..]),
        None => (after_scheme, ""),
    };

    if authority.contains('@') {
        return Err(RelayError::SecretInForbiddenPlace(
            "local seat base_url must not embed userinfo or credentials".to_owned(),
        ));
    }
    if authority.is_empty() {
        return Err(RelayError::SeatConfig(format!(
            "local seat '{guild}' base_url has an empty host"
        )));
    }

    let (host, port_opt) = if let Some(rest) = authority.strip_prefix('[') {
        let Some((ipv6_str, tail)) = rest.split_once(']') else {
            return Err(RelayError::SeatConfig(format!(
                "local seat '{guild}' base_url has unclosed '[' in IPv6 host"
            )));
        };
        let port = if tail.is_empty() {
            None
        } else if let Some(p_str) = tail.strip_prefix(':') {
            let p: u16 = p_str
                .parse::<u16>()
                .ok()
                .filter(|&p| p > 0)
                .ok_or_else(|| {
                    RelayError::SeatConfig(format!(
                        "local seat '{guild}' base_url has an invalid port"
                    ))
                })?;
            Some(p)
        } else {
            return Err(RelayError::SeatConfig(format!(
                "local seat '{guild}' base_url has a malformed authority"
            )));
        };
        (ipv6_str, port)
    } else if let Some((h, p_str)) = authority.rsplit_once(':') {
        if h.contains(':') {
            return Err(RelayError::SeatConfig(format!(
                "local seat '{guild}' IPv6 host in base_url must be enclosed in '[...]'"
            )));
        }
        let p: u16 = p_str
            .parse::<u16>()
            .ok()
            .filter(|&p| p > 0)
            .ok_or_else(|| {
                RelayError::SeatConfig(format!("local seat '{guild}' base_url has an invalid port"))
            })?;
        (h, Some(p))
    } else {
        (authority, None)
    };

    if host.is_empty() {
        return Err(RelayError::SeatConfig(format!(
            "local seat '{guild}' base_url has an empty host"
        )));
    }

    if !is_host_loopback_or_allowed(host, normalized_allowed) {
        return Err(RelayError::SeatConfig(format!(
            "local seat '{guild}' base_url host is refused: must be loopback (127.0.0.1, [::1], localhost) or a VM-private IP passed to --allow-vm-host"
        )));
    }

    let formatted_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_ascii_lowercase()
    };
    let origin = match port_opt {
        Some(port) => format!("{scheme}://{formatted_host}:{port}"),
        None => format!("{scheme}://{formatted_host}"),
    };
    let canonical_host = host
        .parse::<IpAddr>()
        .map_or_else(|_| host.to_ascii_lowercase(), |ip| ip.to_string());
    Ok((origin, canonical_host, raw_path.trim_end_matches('/')))
}

fn looks_like_secret_token(s: &str) -> bool {
    let lower = s.trim().to_ascii_lowercase();
    lower.starts_with("sk-")
        || lower.contains("sk-or-")
        || lower.contains("sk-proj-")
        || lower.starts_with("bearer ")
}

fn contains_forbidden_key_field(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(k, v)| {
            matches!(
                k.to_ascii_lowercase().as_str(),
                "api_key"
                    | "apikey"
                    | "openrouter_api_key"
                    | "openai_api_key"
                    | "vllm_api_key"
                    | "local_api_key"
                    | "secret"
                    | "token"
                    | "bearer"
                    | "authorization"
            ) || contains_forbidden_key_field(v)
        }),
        serde_json::Value::Array(arr) => arr.iter().any(contains_forbidden_key_field),
        serde_json::Value::String(s) => looks_like_secret_token(s),
        _ => false,
    }
}

fn validate_guild_name(guild: &str) -> Result<(), RelayError> {
    let mut chars = guild.chars();
    let Some(first) = chars.next() else {
        return Err(RelayError::SeatConfig(
            "guild name must match [A-Za-z][A-Za-z0-9_-]*".to_owned(),
        ));
    };
    if !first.is_ascii_alphabetic()
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(RelayError::SeatConfig(
            "guild name must match [A-Za-z][A-Za-z0-9_-]*".to_owned(),
        ));
    }
    Ok(())
}

fn is_slug_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
}

/// Validate that a `local` seat's `model` identifier is non-empty, free of whitespace/control
/// characters, not a credential token, and not a floating alias (`auto`, `latest`, `*-latest`).
pub fn validate_pinned_local_model(guild: &str, model: &str) -> Result<(), RelayError> {
    if looks_like_secret_token(model) || model.split('/').any(looks_like_secret_token) {
        return Err(RelayError::SecretInForbiddenPlace(format!(
            "local seat '{guild}' model field looks like an API key or authorization credential"
        )));
    }
    let trimmed = model.trim();
    if trimmed.is_empty()
        || trimmed != model
        || model.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(RelayError::SeatConfig(format!(
            "local seat '{guild}' model must be a non-empty identifier without whitespace"
        )));
    }
    let tail = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    if tail == "auto"
        || tail == "latest"
        || tail.ends_with("-latest")
        || tail.ends_with("_latest")
        || tail.ends_with(".latest")
        || tail.ends_with(":latest")
    {
        return Err(RelayError::SeatConfig(format!(
            "local seat '{guild}' model is a floating alias; pin an exact model id"
        )));
    }
    Ok(())
}

/// Validate that `model` is a pinned `provider/slug` identifier and not a floating alias.
pub fn validate_pinned_openrouter_slug(guild: &str, model: &str) -> Result<(), RelayError> {
    if looks_like_secret_token(model) || model.split('/').any(looks_like_secret_token) {
        return Err(RelayError::SecretInForbiddenPlace(format!(
            "seat '{guild}' model field looks like an API key or authorization credential"
        )));
    }
    let Some((provider, name)) = model.split_once('/') else {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' model must be a pinned OpenRouter 'provider/model' slug"
        )));
    };
    if provider.is_empty()
        || name.is_empty()
        || name.contains('/')
        || name.contains(':')
        || !provider.chars().all(is_slug_char)
        || !name.chars().all(is_slug_char)
    {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' model must be an exact 'provider/model' slug without ':' variants"
        )));
    }
    let lower_name = name.to_ascii_lowercase();
    if lower_name == "auto"
        || lower_name == "latest"
        || lower_name.ends_with("-latest")
        || lower_name.ends_with("_latest")
        || lower_name.ends_with(".latest")
    {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' model is a floating alias; pin an exact model slug"
        )));
    }
    Ok(())
}

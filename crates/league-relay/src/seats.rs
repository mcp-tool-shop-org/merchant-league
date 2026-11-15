//! Seat configuration file parsing and validation.
//!
//! A seat file is JSON (either `{"seats": [...]}` or a top-level `[...]`),
//! listing each seated guild, its backend (`openrouter` or `file`), and for
//! `openrouter` seats its pinned OpenRouter model slug, temperature, maximum
//! output tokens, per-seat wall timeout, and `max_cost_per_call` estimate.

use crate::error::RelayError;
use crate::secret::{ApiKey, redact_secrets};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::Duration;

/// Seat backend selector in the seat configuration file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    /// Automated chat-completions seat called via OpenRouter.
    Openrouter,
    /// Manual seat relayed by hand (for example Claude Code subagents).
    File,
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
    model: Option<String>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    timeout_secs: Option<f64>,
    #[serde(default)]
    max_cost_per_call: Option<f64>,
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
}

/// Validated backend configuration for one seat.
#[derive(Debug, Clone, PartialEq)]
pub enum SeatBackend {
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
    /// Read and validate a JSON seat file from `path`.
    pub fn load(path: &Path, redaction_key: Option<&ApiKey>) -> Result<Self, RelayError> {
        let raw = fs::read_to_string(path).map_err(|e| {
            seat_err(
                format!("could not read {}: {e}", path.display()),
                redaction_key,
            )
        })?;
        Self::from_json_str(&raw, redaction_key)
    }

    /// Parse and validate a JSON seat file from `raw`.
    pub fn from_json_str(raw: &str, redaction_key: Option<&ApiKey>) -> Result<Self, RelayError> {
        let value: serde_json::Value = serde_json::from_str(raw)
            .map_err(|e| seat_err(format!("invalid JSON: {e}"), redaction_key))?;
        if contains_forbidden_key_field(&value) {
            return Err(RelayError::SecretInForbiddenPlace(
                "seat file must not contain an API key; set OPENROUTER_API_KEY in the environment"
                    .to_owned(),
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
        let mut seats = Vec::with_capacity(entries.len());

        for entry in entries {
            validate_guild_name(&entry.guild)?;
            if !seen_guilds.insert(entry.guild.to_ascii_lowercase()) {
                return Err(RelayError::SeatConfig(format!(
                    "duplicate guild '{}' (case-insensitive) in seat file",
                    entry.guild
                )));
            }

            let backend = match entry.backend {
                BackendKind::File => SeatBackend::File { model: entry.model },
                BackendKind::Openrouter => {
                    let model = entry.model.ok_or_else(|| {
                        RelayError::SeatConfig(format!("seat '{}' missing 'model'", entry.guild))
                    })?;
                    validate_pinned_openrouter_slug(&entry.guild, &model)?;

                    let Some(temperature) = entry
                        .temperature
                        .filter(|t| t.is_finite() && (0.0..=2.0).contains(t))
                    else {
                        return Err(RelayError::SeatConfig(format!(
                            "seat '{}' requires finite 'temperature' in 0.0..=2.0",
                            entry.guild
                        )));
                    };

                    let Some(max_output_tokens) = entry.max_output_tokens.filter(|&m| m > 0) else {
                        return Err(RelayError::SeatConfig(format!(
                            "seat '{}' requires positive 'max_output_tokens'",
                            entry.guild
                        )));
                    };

                    let Some(timeout_secs) =
                        entry.timeout_secs.filter(|t| t.is_finite() && *t > 0.0)
                    else {
                        return Err(RelayError::SeatConfig(format!(
                            "seat '{}' requires finite positive 'timeout_secs'",
                            entry.guild
                        )));
                    };

                    let Some(max_cost_per_call) = entry
                        .max_cost_per_call
                        .filter(|c| c.is_finite() && *c > 0.0)
                    else {
                        return Err(RelayError::SeatConfig(format!(
                            "seat '{}' requires finite positive 'max_cost_per_call'",
                            entry.guild
                        )));
                    };

                    SeatBackend::Openrouter(OpenRouterSeat {
                        model,
                        temperature,
                        max_output_tokens,
                        timeout: Duration::from_secs_f64(timeout_secs),
                        max_cost_per_call,
                    })
                }
            };

            seats.push(SeatSpec {
                guild: entry.guild,
                backend,
            });
        }

        Ok(Self { seats })
    }

    /// Whether at least one seat uses the `openrouter` backend.
    pub fn has_openrouter_seat(&self) -> bool {
        self.seats
            .iter()
            .any(|s| matches!(s.backend, SeatBackend::Openrouter(_)))
    }
}

fn contains_forbidden_key_field(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(k, v)| {
            matches!(
                k.to_ascii_lowercase().as_str(),
                "api_key" | "apikey" | "openrouter_api_key" | "secret" | "token" | "authorization"
            ) || contains_forbidden_key_field(v)
        }),
        serde_json::Value::Array(arr) => arr.iter().any(contains_forbidden_key_field),
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

/// Validate that `model` is a pinned `provider/slug` identifier and not a floating alias.
pub fn validate_pinned_openrouter_slug(guild: &str, model: &str) -> Result<(), RelayError> {
    let Some((provider, name)) = model.split_once('/') else {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' model '{model}' must be a pinned OpenRouter 'provider/model' slug"
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
            "seat '{guild}' model '{model}' must be an exact 'provider/model' slug without ':' variants"
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
            "seat '{guild}' model '{model}' is a floating alias; pin an exact model slug"
        )));
    }
    Ok(())
}

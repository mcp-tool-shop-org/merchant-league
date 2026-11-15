//! Seat configuration file parsing and validation.
//!
//! A seat file is JSON (either `{"seats": [...]}` or a top-level `[...]`),
//! listing each seated guild, its backend (`openrouter` or `file`), and for
//! `openrouter` seats its pinned OpenRouter model slug, temperature, maximum
//! output tokens, and per-seat wall timeout.

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
    /// Guild identifier used in `to-seat/`, `from-seat/`, and `relay/<Guild>.jsonl`.
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

impl SeatFile {
    /// Read and validate a JSON seat file from `path`.
    pub fn load(path: &Path, redaction_key: Option<&ApiKey>) -> Result<Self, RelayError> {
        let raw = fs::read_to_string(path).map_err(|e| {
            RelayError::SeatConfig(redact_secrets(
                &format!("could not read {}: {e}", path.display()),
                redaction_key,
            ))
        })?;
        Self::from_json_str(&raw, redaction_key)
    }

    /// Parse and validate a JSON seat file from `raw`.
    pub fn from_json_str(raw: &str, redaction_key: Option<&ApiKey>) -> Result<Self, RelayError> {
        // Reject any embedded key field explicitly before full deserialization so
        // the caller gets a clear secret-placement error with values scrubbed.
        let value: serde_json::Value = serde_json::from_str(raw).map_err(|e| {
            RelayError::SeatConfig(redact_secrets(&format!("invalid JSON: {e}"), redaction_key))
        })?;
        if contains_forbidden_key_field(&value) {
            return Err(RelayError::SecretInForbiddenPlace(
                "seat file must not contain an API key; set OPENROUTER_API_KEY in the environment"
                    .to_owned(),
            ));
        }

        let entries: Vec<SeatEntryWire> = if value.is_array() {
            serde_json::from_value(value).map_err(|e| {
                RelayError::SeatConfig(redact_secrets(&e.to_string(), redaction_key))
            })?
        } else {
            let wrapped: SeatFileWrappedWire = serde_json::from_value(value).map_err(|e| {
                RelayError::SeatConfig(redact_secrets(&e.to_string(), redaction_key))
            })?;
            wrapped.seats
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
            if !seen_guilds.insert(entry.guild.clone()) {
                return Err(RelayError::SeatConfig(format!(
                    "duplicate guild '{}' in seat file",
                    entry.guild
                )));
            }

            let backend = match entry.backend {
                BackendKind::File => SeatBackend::File { model: entry.model },
                BackendKind::Openrouter => {
                    let model = entry.model.ok_or_else(|| {
                        RelayError::SeatConfig(format!(
                            "seat '{}' (openrouter) is missing 'model'",
                            entry.guild
                        ))
                    })?;
                    validate_pinned_openrouter_slug(&entry.guild, &model)?;

                    let temperature = entry.temperature.ok_or_else(|| {
                        RelayError::SeatConfig(format!(
                            "seat '{}' (openrouter) is missing 'temperature'",
                            entry.guild
                        ))
                    })?;
                    if !temperature.is_finite() || !(0.0..=2.0).contains(&temperature) {
                        return Err(RelayError::SeatConfig(format!(
                            "seat '{}' temperature must be finite and in 0.0..=2.0",
                            entry.guild
                        )));
                    }

                    let max_output_tokens = entry.max_output_tokens.ok_or_else(|| {
                        RelayError::SeatConfig(format!(
                            "seat '{}' (openrouter) is missing 'max_output_tokens'",
                            entry.guild
                        ))
                    })?;
                    if max_output_tokens == 0 {
                        return Err(RelayError::SeatConfig(format!(
                            "seat '{}' max_output_tokens must be greater than 0",
                            entry.guild
                        )));
                    }

                    let timeout_secs = entry.timeout_secs.ok_or_else(|| {
                        RelayError::SeatConfig(format!(
                            "seat '{}' (openrouter) is missing 'timeout_secs'",
                            entry.guild
                        ))
                    })?;
                    if !timeout_secs.is_finite() || timeout_secs <= 0.0 {
                        return Err(RelayError::SeatConfig(format!(
                            "seat '{}' timeout_secs must be a finite positive number",
                            entry.guild
                        )));
                    }

                    SeatBackend::Openrouter(OpenRouterSeat {
                        model,
                        temperature,
                        max_output_tokens,
                        timeout: Duration::from_secs_f64(timeout_secs),
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
            let kl = k.to_ascii_lowercase();
            matches!(
                kl.as_str(),
                "api_key" | "apikey" | "openrouter_api_key" | "secret" | "token" | "authorization"
            ) || contains_forbidden_key_field(v)
        }),
        serde_json::Value::Array(arr) => arr.iter().any(contains_forbidden_key_field),
        _ => false,
    }
}

fn validate_guild_name(guild: &str) -> Result<(), RelayError> {
    if guild.is_empty()
        || !guild
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(RelayError::SeatConfig(
            "guild name must be non-empty ASCII alphanumeric, '-' or '_'".to_owned(),
        ));
    }
    Ok(())
}

/// Validate that `model` is a pinned `provider/slug` identifier and not a floating alias.
pub fn validate_pinned_openrouter_slug(guild: &str, model: &str) -> Result<(), RelayError> {
    if model.is_empty() || model.trim() != model || model.chars().any(char::is_whitespace) {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' model slug must be non-empty without whitespace"
        )));
    }
    let Some((provider, name)) = model.split_once('/') else {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' model '{model}' must be a pinned OpenRouter 'provider/model' slug"
        )));
    };
    if provider.is_empty() || name.is_empty() || name.contains('/') {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' model '{model}' must have exact 'provider/model' form"
        )));
    }
    if name.contains(':') {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' model '{model}' uses a ':' routing variant; pin an exact model slug"
        )));
    }
    let lower_full = model.to_ascii_lowercase();
    let lower_name = name.to_ascii_lowercase();
    if lower_full == "openrouter/auto"
        || lower_name == "auto"
        || lower_name == "latest"
        || lower_name.ends_with("-latest")
        || lower_name.ends_with("_latest")
        || lower_name.ends_with(".latest")
    {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' model '{model}' is a floating alias; pin an exact model slug"
        )));
    }
    if !provider
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(RelayError::SeatConfig(format!(
            "seat '{guild}' model '{model}' contains unsupported characters"
        )));
    }
    Ok(())
}

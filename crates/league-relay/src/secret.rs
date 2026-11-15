//! Secret handling, live-call capability gate, and secret redaction.
//!
//! The OpenRouter key is read exclusively from the `OPENROUTER_API_KEY`
//! environment variable. [`ApiKey`] hides its contents in [`fmt::Debug`] and
//! does not implement [`fmt::Display`]. [`LiveGate`] is the only way to build
//! [`UreqTransport`](crate::transport::UreqTransport), guaranteeing at the type
//! level that no HTTP client exists unless `--live`, `--cap-credits`, and
//! `OPENROUTER_API_KEY` are all present.

use crate::error::RelayError;
use std::fmt;

/// Name of the sole environment variable from which the OpenRouter key is read.
pub const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_API_KEY";

/// Placeholder substituted whenever a key or bearer token is scrubbed.
pub const REDACTED_MARKER: &str = "<REDACTED_API_KEY>";

/// An OpenRouter API key read from `OPENROUTER_API_KEY`. Never printed.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    /// Read the key from the process environment (`OPENROUTER_API_KEY`).
    pub fn from_env() -> Result<Self, RelayError> {
        Self::from_env_reader(|name| std::env::var(name).ok())
    }

    /// Read the key through an injected environment lookup (queries only `OPENROUTER_API_KEY`).
    pub fn from_env_reader<F>(reader: F) -> Result<Self, RelayError>
    where
        F: FnOnce(&str) -> Option<String>,
    {
        match reader(OPENROUTER_API_KEY_ENV) {
            Some(raw) if !raw.trim().is_empty() => Ok(Self(raw)),
            _ => Err(RelayError::MissingApiKey),
        }
    }

    pub(crate) fn secret_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

/// Proof token required to construct the live HTTP transport.
pub struct LiveGate {
    api_key: ApiKey,
    cap_credits: f64,
}

impl LiveGate {
    /// Acquire permission for live network calls.
    ///
    /// Refuses unless `live_flag` is `true`, `cap_credits` is a finite non-negative
    /// number, and `api_key` succeeded from `OPENROUTER_API_KEY`.
    pub fn acquire(
        live_flag: bool,
        cap_credits: Option<f64>,
        api_key: Result<ApiKey, RelayError>,
    ) -> Result<Self, RelayError> {
        if !live_flag {
            return Err(RelayError::LiveFlagRequired);
        }
        let Some(cap) = cap_credits.filter(|c| c.is_finite() && *c >= 0.0) else {
            return Err(RelayError::InvalidCap(
                "--cap-credits <n> (finite, non-negative) is required when --live is passed"
                    .to_owned(),
            ));
        };
        Ok(Self {
            api_key: api_key?,
            cap_credits: cap,
        })
    }

    /// Reference to the guarded key for redaction tests and header construction.
    pub fn api_key(&self) -> &ApiKey {
        &self.api_key
    }

    /// Validated credit cap for this live run.
    pub fn cap_credits(&self) -> f64 {
        self.cap_credits
    }
}

impl fmt::Debug for LiveGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveGate")
            .field("api_key", &self.api_key)
            .field("cap_credits", &self.cap_credits)
            .finish()
    }
}

/// Scrub any occurrence of `key`, `Bearer <token>`, or `sk-or-...` tokens from `input`.
pub fn redact_secrets(input: &str, key: Option<&ApiKey>) -> String {
    let scrubbed = match key {
        Some(k) => input.replace(k.secret_str(), REDACTED_MARKER),
        None => input.to_owned(),
    };
    let scrubbed = redact_prefixed_token(&scrubbed, "bearer ", true);
    redact_prefixed_token(&scrubbed, "sk-or-", false)
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
}

fn redact_prefixed_token(input: &str, prefix_lower: &str, keep_prefix: bool) -> String {
    let plen = prefix_lower.len();
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while rest.len() >= plen {
        let Some(idx) = rest
            .as_bytes()
            .windows(plen)
            .position(|w| w.eq_ignore_ascii_case(prefix_lower.as_bytes()))
        else {
            break;
        };
        out.push_str(&rest[..idx]);
        let matched_prefix = &rest[idx..idx + plen];
        let after = rest[idx + plen..].trim_start_matches(' ');
        if let Some(stripped) = after.strip_prefix(REDACTED_MARKER) {
            if keep_prefix {
                out.push_str(matched_prefix);
            }
            out.push_str(REDACTED_MARKER);
            rest = stripped;
            continue;
        }
        let byte_len: usize = after
            .chars()
            .take_while(|&c| is_token_char(c))
            .map(char::len_utf8)
            .sum();
        if keep_prefix {
            out.push_str(matched_prefix);
        }
        if byte_len > 0 || !keep_prefix {
            out.push_str(REDACTED_MARKER);
        }
        rest = &after[byte_len..];
    }
    out.push_str(rest);
    out
}

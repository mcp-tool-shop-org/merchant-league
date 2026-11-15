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
        let Some(cap) = cap_credits else {
            return Err(RelayError::InvalidCap(
                "--cap-credits <n> is required when --live is passed".to_owned(),
            ));
        };
        if !cap.is_finite() || cap < 0.0 {
            return Err(RelayError::InvalidCap(
                "--cap-credits must be a finite non-negative number".to_owned(),
            ));
        }
        let api_key = api_key?;
        Ok(Self {
            api_key,
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
    let mut scrubbed = if let Some(k) = key {
        let raw = k.secret_str();
        if raw.is_empty() {
            input.to_owned()
        } else {
            input.replace(raw, REDACTED_MARKER)
        }
    } else {
        input.to_owned()
    };

    scrubbed = redact_bearer_tokens(&scrubbed);
    scrubbed = redact_openrouter_prefixed_tokens(&scrubbed);
    scrubbed
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
}

fn redact_bearer_tokens(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(idx) = find_ascii_case_insensitive(rest, "bearer ") {
        let prefix_end = idx + "bearer ".len();
        out.push_str(&rest[..prefix_end]);
        let after = &rest[prefix_end..];
        let trimmed = after.trim_start_matches(' ');
        if trimmed.starts_with(REDACTED_MARKER) {
            let marker_end = REDACTED_MARKER.len();
            out.push_str(&trimmed[..marker_end]);
            rest = &trimmed[marker_end..];
            continue;
        }
        let tok_len = trimmed.chars().take_while(|&c| is_token_char(c)).count();
        if tok_len > 0 {
            let byte_len: usize = trimmed.chars().take(tok_len).map(char::len_utf8).sum();
            out.push_str(REDACTED_MARKER);
            rest = &trimmed[byte_len..];
        } else {
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn redact_openrouter_prefixed_tokens(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(idx) = find_ascii_case_insensitive(rest, "sk-or-") {
        out.push_str(&rest[..idx]);
        let after = &rest[idx..];
        let byte_len: usize = after
            .chars()
            .take_while(|&c| is_token_char(c))
            .map(char::len_utf8)
            .sum();
        out.push_str(REDACTED_MARKER);
        rest = &after[byte_len..];
    }
    out.push_str(rest);
    out
}

fn find_ascii_case_insensitive(haystack: &str, needle_lower: &str) -> Option<usize> {
    let nlen = needle_lower.len();
    if haystack.len() < nlen {
        return None;
    }
    haystack
        .as_bytes()
        .windows(nlen)
        .position(|w| w.eq_ignore_ascii_case(needle_lower.as_bytes()))
}

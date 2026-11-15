//! Secret handling, live-call capability gates, and secret redaction.
//!
//! The OpenRouter key is read exclusively from the `OPENROUTER_API_KEY`
//! environment variable. [`ApiKey`] hides its contents in [`fmt::Debug`] and
//! does not implement [`fmt::Display`]. [`LiveGate`] is the only way to build
//! [`UreqTransport`](crate::transport::UreqTransport), and [`LocalGate`] is the
//! only way to build [`LocalOpenAiTransport`](crate::transport::LocalOpenAiTransport)
//! (refusing if any API key is present in the environment so keys never leak
//! into VM logs).

use crate::error::RelayError;
use crate::seats::normalize_allowed_vm_hosts;
use std::fmt;

/// Name of the sole environment variable from which the OpenRouter key is read.
pub const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_API_KEY";

/// Environment variables checked (including common case variants, `C-6`) and refused whenever
/// the local OpenAI-compatible transport runs.
pub const FORBIDDEN_LOCAL_KEY_ENVS: &[&str] = &[
    "OPENROUTER_API_KEY",
    "openrouter_api_key",
    "OPENAI_API_KEY",
    "openai_api_key",
    "VLLM_API_KEY",
    "vllm_api_key",
    "LOCAL_API_KEY",
    "local_api_key",
];

/// Placeholder substituted whenever a key or authorization credential is scrubbed.
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

/// Proof token required to construct the live OpenRouter HTTP transport.
pub struct LiveGate {
    api_key: ApiKey,
    cap_credits: f64,
}

impl LiveGate {
    /// Acquire permission for live OpenRouter network calls.
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

/// Proof token required to construct [`LocalOpenAiTransport`](crate::transport::LocalOpenAiTransport).
///
/// Guarantees at the type level that no API key was supplied in the environment,
/// `--live` was not passed, `--cap-gpu-secs` is finite and non-negative, and every
/// allowed host is loopback or a validated VM-local address.
#[derive(Debug, Clone)]
pub struct LocalGate {
    cap_gpu_secs: f64,
    allowed_vm_hosts: Vec<String>,
}

impl LocalGate {
    /// Acquire permission to call a local OpenAI-compatible server on the VM.
    pub fn acquire(
        live_flag: bool,
        cap_gpu_secs: Option<f64>,
        any_env_key_present: bool,
        allowed_vm_hosts: &[String],
    ) -> Result<Self, RelayError> {
        if any_env_key_present {
            return Err(RelayError::SecretInForbiddenPlace(
                "local transport refuses API keys; unset OPENROUTER_API_KEY, OPENAI_API_KEY, and VLLM_API_KEY so keys never leak into VM logs".to_owned(),
            ));
        }
        if live_flag {
            return Err(RelayError::Usage(
                "--live applies only to OpenRouter seats; omit --live for local seats".to_owned(),
            ));
        }
        let Some(cap) = cap_gpu_secs.filter(|c| c.is_finite() && *c >= 0.0) else {
            return Err(RelayError::InvalidCap(
                "--cap-gpu-secs <n> (finite, non-negative) is required for local seats".to_owned(),
            ));
        };
        let normalized_hosts = normalize_allowed_vm_hosts(allowed_vm_hosts)?;
        Ok(Self {
            cap_gpu_secs: cap,
            allowed_vm_hosts: normalized_hosts,
        })
    }

    /// Validated GPU-seconds ceiling for this local run.
    pub fn cap_gpu_secs(&self) -> f64 {
        self.cap_gpu_secs
    }

    /// Normalized VM-local hosts permitted alongside loopback.
    pub fn allowed_vm_hosts(&self) -> &[String] {
        &self.allowed_vm_hosts
    }
}

/// Upper bound in bytes scanned by [`redact_secrets`] and [`redact_and_truncate`].
///
/// Any tail beyond 4 KiB is discarded before scanning so large upstream HTML
/// error bodies cannot trigger quadratic work while preserving redact-before-truncate.
pub const MAX_REDACTION_SCAN_BYTES: usize = 4096;

/// Upper bound in bytes scanned when formatting [`crate::RelayError::UnrecordedCharge`]
/// on `stderr`, allowing hundreds of held seat lines to print in full with secret redaction.
pub const MAX_UNRECORDED_CHARGE_SCAN_BYTES: usize = 65_536;

const MIN_KEY_SUBSTRING_CHARS: usize = 8;

/// Return the longest UTF-8 prefix of `input` whose byte length is at most `max_bytes`.
pub fn cap_utf8_bytes(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

/// Cap `input` at `max_bytes` and scrub any occurrence of every key in `keys`
/// (including any `8+` character substring), `Bearer <token>`, `sk-or-...`, or `sk-proj-...` tokens (`C-7`).
pub fn redact_secrets_multi_with_limit(input: &str, keys: &[ApiKey], max_bytes: usize) -> String {
    let bounded = cap_utf8_bytes(input, max_bytes);
    let mut scrubbed = bounded.to_owned();
    for k in keys {
        scrubbed = redact_key_substrings(&scrubbed, k.secret_str());
    }
    let scrubbed = redact_prefixed_token(&scrubbed, "bearer ", true);
    let scrubbed = redact_prefixed_token(&scrubbed, "sk-or-", false);
    redact_prefixed_token(&scrubbed, "sk-proj-", false)
}

/// Cap `input` at `max_bytes` and scrub any occurrence of `key` (including any
/// `8+` character substring of `key`), `Bearer <token>`, `sk-or-...`, or `sk-proj-...` tokens.
pub fn redact_secrets_with_limit(input: &str, key: Option<&ApiKey>, max_bytes: usize) -> String {
    match key {
        Some(k) => redact_secrets_multi_with_limit(input, std::slice::from_ref(k), max_bytes),
        None => redact_secrets_multi_with_limit(input, &[], max_bytes),
    }
}

/// Cap `input` at [`MAX_REDACTION_SCAN_BYTES`] (`4 KiB`) and scrub any occurrence
/// of `key` (including any `8+` character substring of `key`), `Bearer <token>`,
/// or `sk-or-...` tokens.
pub fn redact_secrets(input: &str, key: Option<&ApiKey>) -> String {
    redact_secrets_with_limit(input, key, MAX_REDACTION_SCAN_BYTES)
}

/// Cap `input` at [`MAX_REDACTION_SCAN_BYTES`] (`4 KiB`), redact secrets first,
/// and then shorten to at most `max_chars` characters without splitting [`REDACTED_MARKER`].
pub fn redact_and_truncate(input: &str, key: Option<&ApiKey>, max_chars: usize) -> String {
    let bounded = cap_utf8_bytes(input, MAX_REDACTION_SCAN_BYTES);
    let scrubbed = redact_secrets(bounded, key);
    truncate_after_redaction(&scrubbed, max_chars)
}

/// Shorten an already-redacted string to at most `max_chars` characters without
/// splitting [`REDACTED_MARKER`].
pub fn truncate_after_redaction(scrubbed: &str, max_chars: usize) -> String {
    let chars: Vec<char> = scrubbed.chars().collect();
    if chars.len() <= max_chars {
        return scrubbed.to_owned();
    }
    let marker: Vec<char> = REDACTED_MARKER.chars().collect();
    let mlen = marker.len();
    let mut end = max_chars;
    let earliest_start = max_chars.saturating_sub(mlen - 1);
    for start in earliest_start..max_chars {
        if start + mlen <= chars.len() && chars[start..start + mlen] == marker[..] {
            end = start + mlen;
            break;
        }
    }
    chars[..end].iter().collect()
}

/// Install a process-wide panic hook that scrubs `OPENROUTER_API_KEY`,
/// `Bearer ...`, and `sk-or-...` tokens before writing to `stderr`.
pub fn install_redacting_panic_hook(key: Option<ApiKey>) {
    std::panic::set_hook(Box::new(move |info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            *s
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.as_str()
        } else {
            "panic"
        };
        let active_key = key.clone().or_else(|| ApiKey::from_env().ok());
        let scrubbed_msg = redact_secrets(payload, active_key.as_ref());
        let loc_text = match info.location() {
            Some(loc) => redact_secrets(
                &format!("{}:{}:{}", loc.file(), loc.line(), loc.column()),
                active_key.as_ref(),
            ),
            None => "unknown".to_owned(),
        };
        let _ = std::io::Write::write_fmt(
            &mut std::io::stderr(),
            format_args!("panic at {loc_text}: {scrubbed_msg}\n"),
        );
    }));
}

fn redact_key_substrings(input: &str, secret: &str) -> String {
    if secret.is_empty() || input.is_empty() {
        return input.to_owned();
    }
    let secret_chars: Vec<char> = secret.chars().collect();
    if secret_chars.len() < MIN_KEY_SUBSTRING_CHARS {
        return input.replace(secret, REDACTED_MARKER);
    }
    let in_chars: Vec<char> = input.chars().collect();
    let marker_chars: Vec<char> = REDACTED_MARKER.chars().collect();
    let mlen = marker_chars.len();

    let mut redacted = vec![false; in_chars.len()];
    let mut idx = 0;
    while idx < in_chars.len() {
        if idx + mlen <= in_chars.len() && in_chars[idx..idx + mlen] == marker_chars[..] {
            idx += mlen;
            continue;
        }
        if idx + MIN_KEY_SUBSTRING_CHARS <= in_chars.len() {
            let mut best = 0;
            for s in 0..=secret_chars.len() - MIN_KEY_SUBSTRING_CHARS {
                if secret_chars[s] == in_chars[idx] {
                    let common = in_chars[idx..]
                        .iter()
                        .zip(&secret_chars[s..])
                        .take_while(|(a, b)| a == b)
                        .count();
                    if common > best {
                        best = common;
                    }
                }
            }
            if best >= MIN_KEY_SUBSTRING_CHARS {
                for flag in &mut redacted[idx..idx + best] {
                    *flag = true;
                }
            }
        }
        idx += 1;
    }

    let mut out = String::with_capacity(input.len());
    let mut pos = 0;
    while pos < in_chars.len() {
        if redacted[pos] {
            out.push_str(REDACTED_MARKER);
            while pos < in_chars.len() && redacted[pos] {
                pos += 1;
            }
        } else {
            out.push(in_chars[pos]);
            pos += 1;
        }
    }
    out
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

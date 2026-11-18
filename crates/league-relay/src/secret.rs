//! Secret handling, live-call capability gates, log levels, and secret redaction.
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
use std::sync::atomic::{AtomicU8, Ordering};

/// Name of the sole environment variable from which the OpenRouter key is read.
pub const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_API_KEY";

/// Environment variable selecting the default diagnostic log level for `league-relay`.
pub const LEAGUE_LOG_LEVEL_ENV: &str = "LEAGUE_LOG_LEVEL";

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

const EXTRA_SECRET_ENV_NAMES: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "anthropic_api_key",
    "GEMINI_API_KEY",
    "gemini_api_key",
    "LEAGUE_API_KEY",
    "league_api_key",
];

const SECRET_ENV_ASSIGNMENT_PREFIXES: &[&str] = &[
    "OPENROUTER_API_KEY=",
    "ANTHROPIC_API_KEY=",
    "OPENAI_API_KEY=",
    "GEMINI_API_KEY=",
    "LEAGUE_API_KEY=",
    "VLLM_API_KEY=",
    "LOCAL_API_KEY=",
];

const SEALED_STRUCT_PREFIXES: &[&str] = &[
    "SealedEvalSeed",
    "RevealSalt",
    "PreimageSecret",
    "MandateReveal",
    "PrivateMandate",
    "SealedMandateDeal",
];

const SEALED_SECRET_FIELD_KEYS: &[&str] = &[
    "sealed_eval_seed",
    "eval_vrf_seed",
    "eval_vrf_proof",
    "eval_vrf_proofs",
    "reveal_salt",
    "preimage_secret",
    "preimage",
    "fulfillment",
    "mandate_reveal",
    "private_mandate",
    "dealt_mandates",
    "sealed_mandate_deal",
    "salt",
    "xrpl_seed",
    "wallet_seed",
    "family_seed",
    "secret_seed",
    "api_key",
    "authorization",
];

/// Placeholder substituted whenever a key or authorization credential is scrubbed.
pub const REDACTED_MARKER: &str = "<REDACTED_API_KEY>";

/// Diagnostic log verbosity level (`SHIP_GATE.md` `:40`).
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Suppress non-error output and diagnostic logs while still printing scrubbed fatal errors.
    Silent = 0,
    /// Default diagnostic level: prints turn summary paths/hashes and scrubbed fatal errors.
    #[default]
    Normal = 1,
    /// Adds timings and credit/GPU-second cap accounting lines on `stderr`.
    Verbose = 2,
    /// Adds redacted transport, hash, and lock metadata plus redacted panic backtraces (`SHIP_GATE.md` `:28`).
    Debug = 3,
}

impl LogLevel {
    /// Parse `"silent"`, `"normal"`, `"verbose"`, or `"debug"` (case-insensitive).
    pub fn parse(raw: &str) -> Result<Self, RelayError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "silent" => Ok(Self::Silent),
            "normal" => Ok(Self::Normal),
            "verbose" => Ok(Self::Verbose),
            "debug" => Ok(Self::Debug),
            _ => Err(RelayError::Usage(
                "--log-level must be 'silent', 'normal', 'verbose', or 'debug'".to_owned(),
            )),
        }
    }

    /// Lowercase wire and CLI label (`"silent"`, `"normal"`, `"verbose"`, or `"debug"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Silent => "silent",
            Self::Normal => "normal",
            Self::Verbose => "verbose",
            Self::Debug => "debug",
        }
    }

    const fn as_u8(self) -> u8 {
        match self {
            Self::Silent => 0,
            Self::Normal => 1,
            Self::Verbose => 2,
            Self::Debug => 3,
        }
    }

    const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Silent,
            2 => Self::Verbose,
            3 => Self::Debug,
            _ => Self::Normal,
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

static PANIC_HOOK_LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Normal.as_u8());

/// Configure the diagnostic [`LogLevel`] used by the installed panic hook (`SHIP_GATE.md` `:28`).
pub fn set_panic_hook_log_level(level: LogLevel) {
    PANIC_HOOK_LOG_LEVEL.store(level.as_u8(), Ordering::Release);
}

/// Return the active [`LogLevel`] configured for the installed panic hook.
#[must_use]
pub fn panic_hook_log_level() -> LogLevel {
    LogLevel::from_u8(PANIC_HOOK_LOG_LEVEL.load(Ordering::Acquire))
}

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

/// Collect all non-empty secret values present in [`FORBIDDEN_LOCAL_KEY_ENVS`] and provider key envs.
pub fn collect_env_redaction_keys<E>(mut env_reader: E) -> Vec<ApiKey>
where
    E: FnMut(&str) -> Option<String>,
{
    let mut keys: Vec<ApiKey> = Vec::new();
    for env_name in FORBIDDEN_LOCAL_KEY_ENVS
        .iter()
        .chain(EXTRA_SECRET_ENV_NAMES.iter())
    {
        if let Some(val) = env_reader(env_name)
            && !val.trim().is_empty()
            && !keys.iter().any(|existing| existing.secret_str() == val)
        {
            keys.push(ApiKey(val));
        }
    }
    keys
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

/// Cap `input` at `max_bytes` and scrub every secret in `keys` (`8+` char substrings),
/// env-var secret assignments, `Bearer <token>`, `sk-*` API keys, XRPL seeds, and sealed/unrevealed
/// game secrets (`C-7`, `SHIP_GATE.md` `:40`).
pub fn redact_secrets_multi_with_limit(input: &str, keys: &[ApiKey], max_bytes: usize) -> String {
    let bounded = cap_utf8_bytes(input, max_bytes);
    let mut scrubbed = bounded.to_owned();
    for k in keys {
        scrubbed = redact_key_substrings(&scrubbed, k.secret_str());
    }
    scrubbed = redact_env_assignments(&scrubbed);
    scrubbed = redact_prefixed_token(&scrubbed, "bearer ", true);
    scrubbed = redact_prefixed_token(&scrubbed, "sk-or-", false);
    scrubbed = redact_prefixed_token(&scrubbed, "sk-proj-", false);
    scrubbed = redact_prefixed_token(&scrubbed, "sk-ant-", false);
    scrubbed = redact_xrpl_seeds(&scrubbed);
    redact_sealed_and_unrevealed_secrets(&scrubbed)
}

/// Cap `input` at `max_bytes` and scrub any occurrence of `key` (including any
/// `8+` character substring of `key`), env assignments, `Bearer <token>`, API keys,
/// XRPL seeds, and sealed/unrevealed game secrets.
pub fn redact_secrets_with_limit(input: &str, key: Option<&ApiKey>, max_bytes: usize) -> String {
    match key {
        Some(k) => redact_secrets_multi_with_limit(input, std::slice::from_ref(k), max_bytes),
        None => redact_secrets_multi_with_limit(input, &[], max_bytes),
    }
}

/// Cap `input` at [`MAX_REDACTION_SCAN_BYTES`] (`4 KiB`) and scrub secrets.
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

/// Install a process-wide panic hook at [`LogLevel::Normal`] (no stack backtrace) that scrubs
/// `OPENROUTER_API_KEY`, forbidden env keys, `Bearer ...`, API keys, XRPL seeds, and sealed
/// game secrets before writing to `stderr`.
pub fn install_redacting_panic_hook(key: Option<ApiKey>) {
    install_redacting_panic_hook_with_level(key.into_iter().collect(), LogLevel::Normal);
}

/// Install a process-wide panic hook configured with `keys` and `level` (`SHIP_GATE.md` `:28`).
///
/// Emits only a single redacted panic line unless [`panic_hook_log_level`] is [`LogLevel::Debug`],
/// in which case a scrubbed stack backtrace is also printed to `stderr`.
pub fn install_redacting_panic_hook_with_level(keys: Vec<ApiKey>, level: LogLevel) {
    set_panic_hook_log_level(level);
    std::panic::set_hook(Box::new(move |info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            *s
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.as_str()
        } else {
            "panic"
        };
        let mut active_keys = keys.clone();
        for env_key in collect_env_redaction_keys(|name| std::env::var(name).ok()) {
            if !active_keys
                .iter()
                .any(|existing| existing.secret_str() == env_key.secret_str())
            {
                active_keys.push(env_key);
            }
        }
        let scrubbed_msg = redact_secrets_multi_with_limit(
            payload,
            &active_keys,
            MAX_UNRECORDED_CHARGE_SCAN_BYTES,
        );
        let loc_text = match info.location() {
            Some(loc) => redact_secrets_multi_with_limit(
                &format!("{}:{}:{}", loc.file(), loc.line(), loc.column()),
                &active_keys,
                MAX_REDACTION_SCAN_BYTES,
            ),
            None => "unknown".to_owned(),
        };
        let _ = std::io::Write::write_fmt(
            &mut std::io::stderr(),
            format_args!("panic at {loc_text}: {scrubbed_msg}\n"),
        );
        if panic_hook_log_level() == LogLevel::Debug {
            let bt = std::backtrace::Backtrace::force_capture().to_string();
            let scrubbed_bt = redact_secrets_multi_with_limit(
                &bt,
                &active_keys,
                MAX_UNRECORDED_CHARGE_SCAN_BYTES,
            );
            let _ = std::io::Write::write_fmt(
                &mut std::io::stderr(),
                format_args!("stack backtrace:\n{scrubbed_bt}\n"),
            );
        }
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

const fn is_stopper_byte(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b',' | b';' | b')' | b']' | b'}' | b'"' | b'\'')
}

const fn is_base58_byte(b: u8) -> bool {
    matches!(
        b,
        b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'
    )
}

fn starts_with_redacted_marker(rest: &str) -> bool {
    let Some(after) = rest.strip_prefix(REDACTED_MARKER) else {
        return false;
    };
    after.as_bytes().first().is_none_or(|b| is_stopper_byte(*b))
}

fn redact_env_assignments(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        let at_boundary = i == 0 || (!bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_');
        let mut matched_len = None;
        if at_boundary {
            for prefix in SECRET_ENV_ASSIGNMENT_PREFIXES {
                if bytes.len() - i >= prefix.len()
                    && bytes[i..i + prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
                {
                    matched_len = Some(prefix.len());
                    break;
                }
            }
        }
        if let Some(plen) = matched_len {
            out.push_str(&input[i..i + plen]);
            i += plen;
            if starts_with_redacted_marker(&input[i..]) {
                out.push_str(REDACTED_MARKER);
                i += REDACTED_MARKER.len();
                continue;
            }
            let start = i;
            while i < bytes.len() && !is_stopper_byte(bytes[i]) {
                i += 1;
            }
            if i > start {
                out.push_str(REDACTED_MARKER);
            }
            continue;
        }
        let ch = input[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn looks_like_xrpl_seed(slice: &[u8]) -> bool {
    if slice.starts_with(b"sEd") || slice.starts_with(b"sn") {
        return slice.len() >= 22;
    }
    if !(25..=35).contains(&slice.len()) || !slice.starts_with(b"s") {
        return false;
    }
    let has_digit = slice.iter().any(|b| matches!(b, b'1'..=b'9'));
    let has_upper = slice.iter().any(u8::is_ascii_uppercase);
    has_digit && has_upper
}

fn redact_xrpl_seeds(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        let at_boundary = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if at_boundary && bytes[i] == b's' {
            let mut end = i;
            while end < bytes.len() && is_base58_byte(bytes[end]) {
                end += 1;
            }
            let boundary_after = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
            if boundary_after && looks_like_xrpl_seed(&bytes[i..end]) {
                out.push_str(REDACTED_MARKER);
                i = end;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn find_matching_delimited(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    if start >= bytes.len() || bytes[start] != open {
        return None;
    }
    let mut depth = 0usize;
    let mut i = start;
    let mut in_string = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if b == b'"' {
            in_string = true;
        } else if b == open {
            depth += 1;
        } else if b == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn find_json_string_end(bytes: &[u8], quote_start: usize) -> Option<usize> {
    if quote_start >= bytes.len() || bytes[quote_start] != b'"' {
        return None;
    }
    let mut i = quote_start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn redact_sealed_and_unrevealed_secrets(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < bytes.len() {
        let at_word_boundary =
            i == 0 || (!bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_');

        // 1. JSON secret field `"<key>"\s*:\s*<val>`.
        if (i == 0 || bytes[i - 1] != b'\\')
            && bytes[i] == b'"'
            && let Some(key_end) = find_json_string_end(bytes, i)
            && let key = &input[i + 1..key_end]
            && SEALED_SECRET_FIELD_KEYS
                .iter()
                .any(|cand| cand.eq_ignore_ascii_case(key))
        {
            let mut j = key_end + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b':' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() {
                    out.push_str(&input[i..j]);
                    if bytes[j] == b'"'
                        && let Some(val_end) = find_json_string_end(bytes, j)
                    {
                        let inner = &input[j + 1..val_end];
                        out.push('"');
                        if !inner.is_empty() {
                            out.push_str(REDACTED_MARKER);
                        }
                        out.push('"');
                        i = val_end + 1;
                        continue;
                    }
                    if matches!(bytes[j], b'{' | b'[') {
                        let close = if bytes[j] == b'{' { b'}' } else { b']' };
                        if let Some(val_end) = find_matching_delimited(bytes, j, bytes[j], close) {
                            out.push('"');
                            out.push_str(REDACTED_MARKER);
                            out.push('"');
                            i = val_end + 1;
                            continue;
                        }
                    }
                    let mut val_end = j;
                    while val_end < bytes.len() && !is_stopper_byte(bytes[val_end]) {
                        val_end += 1;
                    }
                    if val_end > j {
                        out.push('"');
                        out.push_str(REDACTED_MARKER);
                        out.push('"');
                        i = val_end;
                        continue;
                    }
                }
            }
        }

        // 2. Rust struct debug/display payloads for sealed types (`SealedEvalSeed(...)`, etc.).
        if at_word_boundary {
            let matched_struct = SEALED_STRUCT_PREFIXES
                .iter()
                .find(|prefix| input[i..].starts_with(**prefix));
            if let Some(prefix) = matched_struct {
                let after_name = i + prefix.len();
                let mut j = after_name;
                while j < bytes.len() && bytes[j] == b' ' {
                    j += 1;
                }
                if j < bytes.len() && matches!(bytes[j], b'(' | b'{') {
                    let close = if bytes[j] == b'(' { b')' } else { b'}' };
                    if let Some(end_delim) = find_matching_delimited(bytes, j, bytes[j], close) {
                        let inner = input[j + 1..end_delim].trim();
                        out.push_str(prefix);
                        out.push('(');
                        if inner == "<hidden>" {
                            out.push_str("<hidden>");
                        } else {
                            out.push_str(REDACTED_MARKER);
                        }
                        out.push(')');
                        i = end_delim + 1;
                        continue;
                    }
                }
            }
        }

        // 3. `key=value` secret fields (non-env).
        if at_word_boundary {
            let matched_field = SEALED_SECRET_FIELD_KEYS.iter().find(|key| {
                bytes.len() - i > key.len()
                    && bytes[i..i + key.len()].eq_ignore_ascii_case(key.as_bytes())
                    && bytes[i + key.len()] == b'='
            });
            if let Some(key) = matched_field {
                let eq_after = i + key.len() + 1;
                out.push_str(&input[i..eq_after]);
                if starts_with_redacted_marker(&input[eq_after..]) {
                    out.push_str(REDACTED_MARKER);
                    i = eq_after + REDACTED_MARKER.len();
                    continue;
                }
                let mut end = eq_after;
                while end < bytes.len() && !is_stopper_byte(bytes[end]) {
                    end += 1;
                }
                if end > eq_after {
                    out.push_str(REDACTED_MARKER);
                    i = end;
                    continue;
                }
                i = eq_after;
                continue;
            }
        }

        // 4. Clear sealed-pool `u64` layout seeds (`>= 2^63`), decimal or `0x`-prefixed 64-bit hex.
        if at_word_boundary
            && bytes[i] == b'0'
            && i + 2 < bytes.len()
            && matches!(bytes[i + 1], b'x' | b'X')
        {
            let hex_start = i + 2;
            let mut hex_end = hex_start;
            while hex_end < bytes.len() && bytes[hex_end].is_ascii_hexdigit() {
                hex_end += 1;
            }
            let boundary_after = hex_end == bytes.len()
                || (!bytes[hex_end].is_ascii_alphanumeric() && bytes[hex_end] != b'_');
            if boundary_after
                && hex_end - hex_start == 16
                && matches!(
                    bytes[hex_start],
                    b'8'..=b'9' | b'a'..=b'f' | b'A'..=b'F'
                )
            {
                out.push_str(REDACTED_MARKER);
                i = hex_end;
                continue;
            }
        }
        if at_word_boundary && bytes[i].is_ascii_digit() {
            let mut end = i;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            let boundary_after =
                end == bytes.len() || (!bytes[end].is_ascii_alphanumeric() && bytes[end] != b'_');
            if boundary_after
                && (19..=20).contains(&(end - i))
                && let Ok(val) = input[i..end].parse::<u64>()
                && val >= (1_u64 << 63)
            {
                out.push_str(REDACTED_MARKER);
                i = end;
                continue;
            }
        }

        let ch = input[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }

    out
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

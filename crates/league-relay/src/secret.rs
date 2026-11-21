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

const CREDENTIAL_SECRET_FIELD_KEYS: &[&str] = &[
    "xrpl_seed",
    "wallet_seed",
    "family_seed",
    "secret_seed",
    "api_key",
    "authorization",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum SecretScrubScope {
    AllSecrets,
    CredentialsOnly,
}

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
/// Any tail beyond 4 KiB is discarded (aside from a bounded lookahead when an XRPL family seed,
/// `sk-*` key, `Bearer` token, or env assignment straddles the 4 KiB cut, `B2`) so large
/// upstream HTML error bodies cannot trigger quadratic work while preserving
/// redact-before-truncate and zero partial-seed leakage.
pub const MAX_REDACTION_SCAN_BYTES: usize = 4096;

/// Upper bound in bytes scanned when formatting [`crate::RelayError::UnrecordedCharge`]
/// on `stderr`, allowing hundreds of held seat lines to print in full with secret redaction.
pub const MAX_UNRECORDED_CHARGE_SCAN_BYTES: usize = 65_536;

// Minimum substring length matched when redacting raw `ApiKey` fragments (`C2`).
// If an operator key is split across whitespace or a newline leaving a prefix shorter than
// 8 characters on one side, that `< 8`-character prefix is intentionally left unscrubbed because
// lowering this floor would cause false-positive redactions across normal English words and hex.
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

fn cap_utf8_with_secret_lookahead(input: &str, max_bytes: usize) -> &str {
    const SECRET_LOOKAHEAD_BYTES: usize = 96;
    let cut = cap_utf8_bytes(input, max_bytes).len();
    if cut == input.len() {
        return input;
    }
    let window = cap_utf8_bytes(input, max_bytes.saturating_add(SECRET_LOOKAHEAD_BYTES));
    let bytes = window.as_bytes();
    let mut effective_cut = cut;

    // 1. XRPL family seed starting < cut and straddling past cut (B2 / PR #80 B1).
    let mut b58_start = cut;
    while b58_start > 0 && league_error::is_base58_byte(bytes[b58_start - 1]) {
        b58_start -= 1;
    }
    if let Some(seed_start) =
        (b58_start..cut).find(|&p| bytes[p] == b's' && league_error::is_at_boundary(bytes, p))
    {
        let mut b58_end = cut;
        while b58_end < bytes.len() && league_error::is_base58_byte(bytes[b58_end]) {
            b58_end += 1;
        }
        let boundary_after = b58_end == bytes.len() || !bytes[b58_end].is_ascii_alphanumeric();
        if b58_end > cut
            && boundary_after
            && league_error::looks_like_xrpl_family_seed(&bytes[seed_start..b58_end])
        {
            effective_cut = effective_cut.max(b58_end);
        }
    }

    // 2. Case-insensitive `sk-or-` / `sk-proj-` / `sk-ant-` token starting anywhere < cut and straddling past cut.
    let tok_scan_from = cut.saturating_sub(48);
    for cand in tok_scan_from..cut {
        let plen_opt = [
            b"sk-or-".as_slice(),
            b"sk-proj-".as_slice(),
            b"sk-ant-".as_slice(),
        ]
        .into_iter()
        .find(|p| {
            bytes.len() - cand >= p.len() && bytes[cand..cand + p.len()].eq_ignore_ascii_case(p)
        })
        .map(<[u8]>::len);
        if let Some(plen) = plen_opt {
            let mut end = cand + plen;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'-' | b'_' | b'.'))
            {
                end += 1;
            }
            if end > cut {
                effective_cut = effective_cut.max(end);
                break;
            }
        }
    }

    // 3. Case-insensitive `Bearer <tok>` / `Bearer\t<tok>` anywhere or boundary-gated
    //    `ENV_KEY=<val>` starting < cut whose value straddles cut.
    let scan_from = cut.saturating_sub(48);
    for pos in scan_from..cut {
        if !window.is_char_boundary(pos) {
            continue;
        }
        let val_start_opt = if pos + 7 <= bytes.len()
            && bytes[pos..pos + 6].eq_ignore_ascii_case(b"bearer")
            && matches!(bytes[pos + 6], b' ' | b'\t')
        {
            let mut p = pos + 6;
            while p < bytes.len() && matches!(bytes[p], b' ' | b'\t') {
                p += 1;
            }
            Some(p)
        } else if league_error::is_at_boundary(bytes, pos) {
            SECRET_ENV_ASSIGNMENT_PREFIXES.iter().find_map(|prefix| {
                (bytes.len() - pos >= prefix.len()
                    && bytes[pos..pos + prefix.len()].eq_ignore_ascii_case(prefix.as_bytes()))
                .then_some(pos + prefix.len())
            })
        } else {
            None
        };
        if let Some(val_start) = val_start_opt
            && val_start <= cut
        {
            let mut val_end = val_start;
            while val_end < bytes.len() && !league_error::is_token_stopper_at(bytes, val_end) {
                val_end += 1;
            }
            if val_end > cut {
                effective_cut = effective_cut.max(val_end);
            }
        }
    }

    &window[..effective_cut]
}

const MAX_JSON_POINTER_STRING_DEPTH: usize = 8;

pub(crate) fn is_sealed_secret_field_key(key: &str) -> bool {
    SEALED_SECRET_FIELD_KEYS
        .iter()
        .any(|cand| cand.eq_ignore_ascii_case(key))
}

pub(crate) fn is_credential_secret_field_key(key: &str) -> bool {
    CREDENTIAL_SECRET_FIELD_KEYS
        .iter()
        .any(|cand| cand.eq_ignore_ascii_case(key))
}

fn matches_secret_field_key(key: &str, scope: SecretScrubScope) -> bool {
    match scope {
        SecretScrubScope::AllSecrets => is_sealed_secret_field_key(key),
        SecretScrubScope::CredentialsOnly => is_credential_secret_field_key(key),
    }
}

fn escape_json_pointer_token(token: &str) -> String {
    if !token.contains('~') && !token.contains('/') {
        return token.to_owned();
    }
    let mut out = String::with_capacity(token.len() + 4);
    for ch in token.chars() {
        match ch {
            '~' => out.push_str("~0"),
            '/' => out.push_str("~1"),
            other => out.push(other),
        }
    }
    out
}

fn json_secret_value_would_be_blanked(val: &serde_json::Value) -> bool {
    match val {
        serde_json::Value::String(s) => !s.is_empty() && s != REDACTED_MARKER,
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::Array(_)
        | serde_json::Value::Object(_) => true,
    }
}

fn collect_secret_field_json_paths(
    val: &serde_json::Value,
    prefix: &str,
    string_depth: usize,
    scope: SecretScrubScope,
    out: &mut Vec<String>,
) {
    match val {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let path = format!("{prefix}/{}", escape_json_pointer_token(k));
                if matches_secret_field_key(k, scope) {
                    if json_secret_value_would_be_blanked(v) {
                        out.push(path);
                    }
                    // Branch 1 skips past a matched secret object/array/string rather than keeping
                    // inner keys separately, so do not descend into a blanked secret value.
                    continue;
                }
                collect_secret_field_json_paths(v, &path, string_depth, scope, out);
            }
        }
        serde_json::Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                let path = format!("{prefix}/{idx}");
                collect_secret_field_json_paths(item, &path, string_depth, scope, out);
            }
        }
        serde_json::Value::String(s) if string_depth < MAX_JSON_POINTER_STRING_DEPTH => {
            let trimmed = s.trim_start();
            if (trimmed.starts_with('{') || trimmed.starts_with('['))
                && let Ok(nested) = serde_json::from_str::<serde_json::Value>(s)
                && matches!(
                    nested,
                    serde_json::Value::Object(_) | serde_json::Value::Array(_)
                )
            {
                collect_secret_field_json_paths(&nested, prefix, string_depth + 1, scope, out);
            }
        }
        _ => {}
    }
}

fn redact_secrets_multi_core(
    input: &str,
    keys: &[ApiKey],
    max_bytes: usize,
    collect_paths: bool,
    scope: SecretScrubScope,
) -> (String, Vec<String>) {
    let bounded = cap_utf8_with_secret_lookahead(input, max_bytes);
    let mut raw_paths: Vec<String> = Vec::new();
    let parsed_bounded = if collect_paths {
        Some(serde_json::from_str::<serde_json::Value>(bounded))
    } else {
        None
    };
    if let Some(Ok(ref val)) = parsed_bounded {
        collect_secret_field_json_paths(val, "", 0, scope, &mut raw_paths);
    }

    let mut scrubbed = bounded.to_owned();
    for k in keys {
        scrubbed = redact_key_substrings(&scrubbed, k.secret_str());
    }
    scrubbed = league_error::scrub_secret_text_with_marker(&scrubbed, REDACTED_MARKER);
    let (final_scrubbed, branch1_keys) =
        redact_sealed_and_unrevealed_secrets_inner(&scrubbed, scope);
    if !collect_paths {
        return (final_scrubbed, Vec::new());
    }

    let candidate_paths = if matches!(parsed_bounded, Some(Ok(_))) {
        raw_paths
    } else {
        branch1_keys
            .into_iter()
            .map(|k| format!("/{}", escape_json_pointer_token(&k)))
            .collect::<Vec<_>>()
    };

    let mut deduped = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for p in candidate_paths {
        let safe_path = redact_secrets_multi_core(&p, keys, p.len(), false, scope).0;
        if seen.insert(safe_path.clone()) {
            deduped.push(safe_path);
        }
    }
    (final_scrubbed, deduped)
}

/// Cap `input` at `max_bytes` (with secret lookahead for straddling tokens, `B2`), scrub
/// credentials ONLY (`ApiKey` substrings, env-var/Bearer/`sk-*`/XRPL-seed credential patterns, and
/// [`CREDENTIAL_SECRET_FIELD_KEYS`]) while preserving game values, and return deduplicated
/// RFC 6901 JSON Pointer paths of credential-named fields whose non-empty values were blanked.
pub fn redact_secrets_multi_with_limit_and_paths(
    input: &str,
    keys: &[ApiKey],
    max_bytes: usize,
) -> (String, Vec<String>) {
    redact_secrets_multi_core(
        input,
        keys,
        max_bytes,
        true,
        SecretScrubScope::CredentialsOnly,
    )
}

/// Cap `input` at `max_bytes` (with secret lookahead for straddling tokens, `B2`) and scrub
/// credentials ONLY (`ApiKey` substrings, env-var/Bearer/`sk-*`/XRPL-seed credential patterns, and
/// [`CREDENTIAL_SECRET_FIELD_KEYS`]) while leaving game values as written.
pub(crate) fn redact_credentials_multi_with_limit(
    input: &str,
    keys: &[ApiKey],
    max_bytes: usize,
) -> String {
    redact_secrets_multi_core(
        input,
        keys,
        max_bytes,
        false,
        SecretScrubScope::CredentialsOnly,
    )
    .0
}

/// Cap `input` at `max_bytes` (with secret lookahead for straddling seeds/tokens, `B2`) and scrub
/// every secret in `keys` (`8+` char substrings), env-var secret assignments, `Bearer <token>` /
/// `Bearer\t<token>` (`C1`), `sk-*` API keys, XRPL seeds (via [`league_error`]), and
/// sealed/unrevealed game secrets including JSON-escaped fields (`B1`, `C-7`, `SHIP_GATE.md` `:40`).
pub fn redact_secrets_multi_with_limit(input: &str, keys: &[ApiKey], max_bytes: usize) -> String {
    redact_secrets_multi_core(input, keys, max_bytes, false, SecretScrubScope::AllSecrets).0
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

/// Cap `input` at [`MAX_REDACTION_SCAN_BYTES`] (`4 KiB`, with straddling-secret lookahead),
/// redact secrets first, and then shorten to at most `max_chars` characters without splitting
/// [`REDACTED_MARKER`].
pub fn redact_and_truncate(input: &str, key: Option<&ApiKey>, max_chars: usize) -> String {
    let scrubbed = redact_secrets(input, key);
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

// Substring matcher for `ApiKey` fragments of at least `MIN_KEY_SUBSTRING_CHARS` (`8`).
// When an operator key is split across whitespace/newlines leaving a fragment shorter than
// 8 chars, that fragment stays unredacted by design (`C2`).
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

const fn is_stopper_byte(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b',' | b';' | b')' | b']' | b'}' | b'"' | b'\'' | b'\\')
}

fn starts_with_redacted_marker(rest: &str) -> bool {
    let Some(after) = rest.strip_prefix(REDACTED_MARKER) else {
        return false;
    };
    league_error::is_token_stopper_at(after.as_bytes(), 0)
}

fn count_preceding_backslashes(bytes: &[u8], idx: usize) -> usize {
    let mut k = idx;
    let mut count = 0usize;
    while k > 0 && bytes[k - 1] == b'\\' {
        count += 1;
        k -= 1;
    }
    count
}

fn find_escaped_json_string_end_with_bs(
    bytes: &[u8],
    quote_start: usize,
    bs_expected: usize,
) -> Option<usize> {
    if quote_start + bs_expected >= bytes.len()
        || !bytes[quote_start..quote_start + bs_expected]
            .iter()
            .all(|&b| b == b'\\')
        || bytes[quote_start + bs_expected] != b'"'
    {
        return None;
    }
    let period = 2 * (bs_expected + 1);
    let mut i = quote_start + bs_expected + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let mut bs = 1usize;
            while i + bs < bytes.len() && bytes[i + bs] == b'\\' {
                bs += 1;
            }
            if i + bs < bytes.len() && bytes[i + bs] == b'"' {
                if bs < bs_expected && bs % 2 == 1 {
                    // An outer-level escaped quote terminated before this inner string closed.
                    return None;
                }
                if bs >= bs_expected && (bs - bs_expected).is_multiple_of(period) {
                    return Some(i + bs - bs_expected);
                }
                i += bs + 1;
                continue;
            }
            i += bs;
            continue;
        }
        if bs_expected > 0 && bytes[i] == b'"' {
            // Unescaped quote ended the enclosing string before the escaped inner string closed.
            return None;
        }
        i += 1;
    }
    None
}

fn match_field_quote_open(bytes: &[u8], i: usize) -> Option<(usize, usize, usize)> {
    let preceding = count_preceding_backslashes(bytes, i);
    if preceding.is_multiple_of(2) && bytes[i] == b'"' {
        return Some((i + 1, 1, 0));
    }
    if preceding == 0 && bytes[i] == b'\\' {
        let mut bs = 1usize;
        while i + bs < bytes.len() && bytes[i + bs] == b'\\' {
            bs += 1;
        }
        if bs % 2 == 1 && bytes.get(i + bs) == Some(&b'"') {
            return Some((i + bs + 1, bs + 1, bs));
        }
    }
    None
}

fn find_matching_delimited(
    bytes: &[u8],
    start: usize,
    open: u8,
    close: u8,
    quote_bs: usize,
) -> Option<usize> {
    if start >= bytes.len() || bytes[start] != open {
        return None;
    }
    let mut depth = 0usize;
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if quote_bs == 0 && b == b'"' && count_preceding_backslashes(bytes, i).is_multiple_of(2) {
            let end = find_json_string_end(bytes, i)?;
            i = end + 1;
            continue;
        }
        if quote_bs > 0 && b == b'\\' && count_preceding_backslashes(bytes, i) == 0 {
            let mut bs = 1usize;
            while i + bs < bytes.len() && bytes[i + bs] == b'\\' {
                bs += 1;
            }
            if bs == quote_bs
                && bytes.get(i + bs) == Some(&b'"')
                && let Some(end) = find_escaped_json_string_end_with_bs(bytes, i, quote_bs)
            {
                i = end + quote_bs + 1;
                continue;
            }
            i += bs;
            continue;
        }
        if b == open {
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

fn redact_sealed_and_unrevealed_secrets_inner(
    input: &str,
    scope: SecretScrubScope,
) -> (String, Vec<String>) {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut blanked_keys = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let at_word_boundary =
            league_error::is_at_boundary(bytes, i) && (i == 0 || bytes[i - 1] != b'_');

        // 1. JSON secret field `"<key>"\s*:\s*<val>` and JSON-escaped `\"<key>\"\s*:\s*<val>` (B1/B2).
        //    Never emit `&input[i..j]` until a branch commits, and redact unterminated values
        //    to the end of the input so keys are never duplicated and no secret byte leaks.
        if let Some((key_start, quote_len, quote_bs)) = match_field_quote_open(bytes, i) {
            let key_close_opt = if quote_bs > 0 {
                find_escaped_json_string_end_with_bs(bytes, i, quote_bs)
            } else {
                find_json_string_end(bytes, i)
            };
            if let Some(key_end) = key_close_opt
                && let key = &input[key_start..key_end]
                && matches_secret_field_key(key, scope)
            {
                let mut j = key_end + quote_len;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b':' {
                    j += 1;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    let quote_seq = &input[i..i + quote_len];
                    if j == bytes.len() {
                        out.push_str(&input[i..j]);
                        i = j;
                        continue;
                    }
                    if quote_bs == 0 && bytes[j] == b'"' {
                        out.push_str(&input[i..j]);
                        out.push('"');
                        if let Some(val_end) = find_json_string_end(bytes, j) {
                            let inner = &input[j + 1..val_end];
                            if !inner.is_empty() {
                                out.push_str(REDACTED_MARKER);
                                if inner != REDACTED_MARKER {
                                    blanked_keys.push(key.to_owned());
                                }
                            }
                            out.push('"');
                            i = val_end + 1;
                        } else {
                            if j + 1 < bytes.len() {
                                out.push_str(REDACTED_MARKER);
                                blanked_keys.push(key.to_owned());
                            }
                            i = bytes.len();
                        }
                        continue;
                    }
                    if quote_bs > 0
                        && bytes.len() - j > quote_bs
                        && bytes[j..j + quote_bs].iter().all(|&b| b == b'\\')
                        && bytes[j + quote_bs] == b'"'
                    {
                        out.push_str(&input[i..j]);
                        out.push_str(quote_seq);
                        if let Some(val_end) =
                            find_escaped_json_string_end_with_bs(bytes, j, quote_bs)
                        {
                            let inner = &input[j + quote_len..val_end];
                            if !inner.is_empty() {
                                out.push_str(REDACTED_MARKER);
                                if inner != REDACTED_MARKER {
                                    blanked_keys.push(key.to_owned());
                                }
                            }
                            out.push_str(quote_seq);
                            i = val_end + quote_len;
                        } else {
                            if j + quote_len < bytes.len() {
                                out.push_str(REDACTED_MARKER);
                                blanked_keys.push(key.to_owned());
                            }
                            i = bytes.len();
                        }
                        continue;
                    }
                    if matches!(bytes[j], b'"' | b'\'' | b'\\') {
                        out.push_str(&input[i..j]);
                        out.push_str(quote_seq);
                        out.push_str(REDACTED_MARKER);
                        blanked_keys.push(key.to_owned());
                        i = bytes.len();
                        continue;
                    }
                    if matches!(bytes[j], b'{' | b'[') {
                        let close = if bytes[j] == b'{' { b'}' } else { b']' };
                        out.push_str(&input[i..j]);
                        out.push_str(quote_seq);
                        out.push_str(REDACTED_MARKER);
                        blanked_keys.push(key.to_owned());
                        if let Some(val_end) =
                            find_matching_delimited(bytes, j, bytes[j], close, quote_bs)
                        {
                            out.push_str(quote_seq);
                            i = val_end + 1;
                        } else {
                            i = bytes.len();
                        }
                        continue;
                    }
                    if starts_with_redacted_marker(&input[j..]) {
                        out.push_str(&input[i..j]);
                        out.push_str(REDACTED_MARKER);
                        i = j + REDACTED_MARKER.len();
                        continue;
                    }
                    let mut val_end = j;
                    while val_end < bytes.len() && !is_stopper_byte(bytes[val_end]) {
                        val_end += 1;
                    }
                    out.push_str(&input[i..j]);
                    if val_end > j {
                        out.push_str(quote_seq);
                        out.push_str(REDACTED_MARKER);
                        out.push_str(quote_seq);
                        blanked_keys.push(key.to_owned());
                    }
                    i = val_end;
                    continue;
                }
            }
        }

        // 2. Rust struct debug/display payloads for sealed types (`SealedEvalSeed(...)`, etc.).
        if scope == SecretScrubScope::AllSecrets && at_word_boundary {
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
                    if let Some(end_delim) = find_matching_delimited(bytes, j, bytes[j], close, 0) {
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
                    out.push_str(prefix);
                    out.push('(');
                    if j + 1 < bytes.len() {
                        out.push_str(REDACTED_MARKER);
                    }
                    i = bytes.len();
                    continue;
                }
            }
        }

        // 3. `key=value` secret fields (non-env).
        if at_word_boundary {
            let field_keys = match scope {
                SecretScrubScope::AllSecrets => SEALED_SECRET_FIELD_KEYS,
                SecretScrubScope::CredentialsOnly => CREDENTIAL_SECRET_FIELD_KEYS,
            };
            let matched_field = field_keys.iter().find(|key| {
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
                while end < bytes.len() && !league_error::is_token_stopper_at(bytes, end) {
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
        if scope == SecretScrubScope::AllSecrets
            && at_word_boundary
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
        if scope == SecretScrubScope::AllSecrets && at_word_boundary && bytes[i].is_ascii_digit() {
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

    (out, blanked_keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b1_json_escaped_preimage_secret_and_non_sk_api_key_redacted() {
        let escaped_preimage = r#"line {\"preimage_secret\":\"leakval9988\"} end"#;
        let scrubbed_preimage = redact_secrets(escaped_preimage, None);
        assert!(
            !scrubbed_preimage.contains("leakval9988"),
            "escaped preimage_secret leaked: {scrubbed_preimage}"
        );
        assert_eq!(
            scrubbed_preimage,
            r#"line {\"preimage_secret\":\"<REDACTED_API_KEY>\"} end"#
        );

        let escaped_api_key = r#"outer {\"api_key\":\"plainNonSkSecret42\"} tail"#;
        let scrubbed_api_key = redact_secrets(escaped_api_key, None);
        assert!(
            !scrubbed_api_key.contains("plainNonSkSecret42"),
            "escaped non-sk api_key leaked: {scrubbed_api_key}"
        );
        assert_eq!(
            scrubbed_api_key,
            r#"outer {\"api_key\":\"<REDACTED_API_KEY>\"} tail"#
        );

        // Nested serialized JSON string remains valid JSON after redaction.
        let inner = serde_json::json!({
            "preimage_secret": "nestedLeak777",
            "api_key": "plainInnerNonSkKey"
        })
        .to_string();
        let outer = serde_json::json!({ "err": inner }).to_string();
        let scrubbed_outer = redact_secrets(&outer, None);
        assert!(!scrubbed_outer.contains("nestedLeak777"));
        assert!(!scrubbed_outer.contains("plainInnerNonSkKey"));
        let reparsed_outer: serde_json::Value = serde_json::from_str(&scrubbed_outer)
            .unwrap_or_else(|e| panic!("outer JSON broken: {e}: {scrubbed_outer}"));
        let reparsed_inner: serde_json::Value = reparsed_outer["err"]
            .as_str()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_else(|| panic!("inner JSON broken: {reparsed_outer}"));
        assert_eq!(reparsed_inner["preimage_secret"], REDACTED_MARKER);
        assert_eq!(reparsed_inner["api_key"], REDACTED_MARKER);

        // Escaped nested JSON object with `}` inside string value, and double-escaped JSON (`\\\"`).
        let escaped_obj =
            r#"wrap {\"private_mandate\":{\"note\":\"x}\",\"secret\":\"nestedObjLeak99\"}} end"#;
        let scrubbed_obj = redact_secrets(escaped_obj, None);
        assert!(!scrubbed_obj.contains("nestedObjLeak99"), "{scrubbed_obj}");
        assert!(
            scrubbed_obj.ends_with("} end"),
            "outer escaped object closing brace eaten: {scrubbed_obj}"
        );

        let double_escaped = r#"lvl2 \\\"preimage_secret\\\":\\\"doubleEscapedLeak11\\\" end"#;
        let scrubbed_double = redact_secrets(double_escaped, None);
        assert!(
            !scrubbed_double.contains("doubleEscapedLeak11"),
            "{scrubbed_double}"
        );
        assert_eq!(
            scrubbed_double,
            r#"lvl2 \\\"preimage_secret\\\":\\\"<REDACTED_API_KEY>\\\" end"#
        );
    }

    #[test]
    fn b2_xrpl_seed_straddling_4kib_cap_offsets_1_to_30_leaves_zero_partial_seed() {
        let ed_seed = "sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r";
        let sn_seed = "sn3nAkU7v8w9xPy1z2A3B4C5D6E7F";
        let secp_seed = "sp5fghtJtpUorTwvof1NpDXAzNwf5";
        let contexts: [(&str, &str); 10] = [
            ("space", " "),
            ("colon", ":"),
            ("esc_quote", "\\\""),
            ("esc_nl", "\\n"),
            ("esc_tab", "\\t"),
            ("esc_cr", "\\r"),
            ("pct_3d", "%3D"),
            ("esc_b", "\\b"),
            ("esc_f", "\\f"),
            ("esc_u000a", "\\u000a"),
        ];
        for (seed_name, seed) in [("ed", ed_seed), ("sn", sn_seed), ("secp", secp_seed)] {
            for (ctx_name, delim) in contexts {
                for offset in 1..=30_usize {
                    let prefix_len = MAX_REDACTION_SCAN_BYTES - offset;
                    let pad_len = prefix_len - delim.len();
                    let body = format!("{}{delim}{seed} trailing_payload", "a".repeat(pad_len));
                    for scrubbed in [
                        redact_secrets(&body, None),
                        redact_and_truncate(&body, None, MAX_REDACTION_SCAN_BYTES + 64),
                    ] {
                        assert!(
                            !scrubbed.contains("sEd")
                                && !scrubbed.contains("sn3n")
                                && !scrubbed.contains("sp5f"),
                            "{seed_name} {ctx_name} offset {offset}: partial seed prefix survived: tail={:?}",
                            &scrubbed[scrubbed.len().saturating_sub(48)..]
                        );
                        assert!(
                            !scrubbed.contains(&seed[..4]),
                            "{seed_name} {ctx_name} offset {offset}: partial seed survived in {scrubbed:?}"
                        );
                        assert!(
                            scrubbed.contains(REDACTED_MARKER),
                            "{seed_name} {ctx_name} offset {offset}: expected redaction marker in {scrubbed:?}"
                        );
                        if offset <= seed.len() {
                            assert!(
                                scrubbed.ends_with(REDACTED_MARKER),
                                "{seed_name} {ctx_name} offset {offset}: expected marker at cap, got tail={:?}",
                                &scrubbed[scrubbed.len().saturating_sub(48)..]
                            );
                        }
                    }
                }
                println!(
                    "CAP_SWEEP seed={seed_name} ctx={ctx_name} offsets=1..=30 partial_leaks=0"
                );
            }
        }
    }

    #[test]
    fn c1_bearer_followed_by_tab_is_redacted() {
        let scrubbed = redact_secrets("Authorization: Bearer\ttab_secret_token_999\tend", None);
        assert!(!scrubbed.contains("tab_secret_token_999"), "{scrubbed}");
        assert_eq!(scrubbed, "Authorization: Bearer\t<REDACTED_API_KEY>\tend");
    }

    #[test]
    fn b2_json_field_matcher_never_duplicates_key_or_leaks_unterminated_values() {
        // 1. Unterminated JSON field value -> key once, secret completely redacted to end.
        let unterminated = r#"a "api_key":"leakUnterm1"#;
        let out_unterm = redact_secrets(unterminated, None);
        assert_eq!(out_unterm.matches("api_key").count(), 1, "{out_unterm}");
        assert!(!out_unterm.contains("leakUnterm1"), "{out_unterm}");
        assert_eq!(out_unterm, r#"a "api_key":"<REDACTED_API_KEY>"#);
        assert_eq!(redact_secrets(&out_unterm, None), out_unterm);

        // 2. Empty value before delimiter -> key once, no duplication.
        let empty_val = r#"a "api_key": , b"#;
        let out_empty = redact_secrets(empty_val, None);
        assert_eq!(out_empty.matches("api_key").count(), 1, "{out_empty}");
        assert_eq!(out_empty, empty_val);
        assert_eq!(redact_secrets(&out_empty, None), out_empty);

        // 3. Mismatched escaped open / unescaped close quote -> key once, no secret byte leaked.
        let escaped_mismatch = r#"a {\"preimage_secret\":\"leakPQ3"} b"#;
        let out_mismatch = redact_secrets(escaped_mismatch, None);
        assert_eq!(
            out_mismatch.matches("preimage_secret").count(),
            1,
            "{out_mismatch}"
        );
        assert!(!out_mismatch.contains("leakPQ3"), "{out_mismatch}");
        assert_eq!(
            out_mismatch,
            r#"a {\"preimage_secret\":\"<REDACTED_API_KEY>"#
        );
        assert_eq!(redact_secrets(&out_mismatch, None), out_mismatch);

        // 4. `"api_key":"..."` value cut at every offset across the 4 KiB cap -> key once, 0 secret bytes.
        let secret_val = "12345678901234567890123456";
        let field = format!(r#""api_key":"{secret_val}""#);
        for offset in 1..=(field.len() + 2) {
            let prefix_len = MAX_REDACTION_SCAN_BYTES - offset;
            let body = format!("{}{field} tail", "z".repeat(prefix_len));
            let scrubbed = redact_secrets(&body, None);
            if body[..MAX_REDACTION_SCAN_BYTES].contains("api_key") {
                assert_eq!(
                    scrubbed.matches("api_key").count(),
                    1,
                    "offset {offset}: key duplicated or dropped: {:?}",
                    &scrubbed[scrubbed.len().saturating_sub(64)..]
                );
            }
            for digit in secret_val.chars() {
                assert!(
                    !scrubbed.contains(digit),
                    "offset {offset}: secret digit '{digit}' survived: {:?}",
                    &scrubbed[scrubbed.len().saturating_sub(64)..]
                );
            }
            assert!(
                !scrubbed.contains(secret_val),
                "offset {offset}: secret survived: {:?}",
                &scrubbed[scrubbed.len().saturating_sub(64)..]
            );
            assert_eq!(redact_secrets(&scrubbed, None), scrubbed);
        }
    }

    #[test]
    fn card9_should8_redacted_secret_field_paths_tracks_nested_and_escaped_json_without_values() {
        let key = ApiKey("sk-or-v1-live-secret-key-987654".to_owned());
        let payload = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "SealedEvalSeed(99) 0x8000000000000001",
                    "tool_calls": [{
                        "function": {
                            "name": "preview_turn_orders",
                            "arguments": "{\"preimage\":\"0xsecret_preimage_val\",\"salt\":\"GameSaltKeep\",\"nested\":{\"xrpl_seed\":\"sEdSecretSeedValue\"}}"
                        }
                    }]
                }
            }],
            "api_key": "sk-or-v1-live-secret-key-987654",
            "empty_salt": "",
            "already_clean": REDACTED_MARKER,
            "authorization": "",
            "non_secret_with_key": "sk-or-v1-live-secret-key-987654"
        })
        .to_string();

        let (scrubbed, paths) = redact_secrets_multi_with_limit_and_paths(
            &payload,
            std::slice::from_ref(&key),
            payload.len(),
        );
        assert_eq!(
            redact_credentials_multi_with_limit(
                &payload,
                std::slice::from_ref(&key),
                payload.len(),
            ),
            scrubbed
        );
        assert_eq!(
            paths,
            vec![
                "/api_key",
                "/choices/0/message/tool_calls/0/function/arguments/nested/xrpl_seed",
            ]
        );
        // Credentials blanked and omitted from paths.
        for cred in ["sEdSecretSeedValue", "987654"] {
            assert!(!scrubbed.contains(cred));
            for p in &paths {
                assert!(!p.contains(cred), "path leaked value: {p}");
            }
        }
        // Game values remain intact in credential-only kept-model-output scrubbing.
        for game_val in [
            "0xsecret_preimage_val",
            "GameSaltKeep",
            "SealedEvalSeed(99)",
            "0x8000000000000001",
        ] {
            assert!(
                scrubbed.contains(game_val),
                "credential-only scrubber must keep game value `{game_val}`: {scrubbed}"
            );
        }

        // Full log/error redaction STILL blanks all `SEALED_SECRET_FIELD_KEYS` & sealed types.
        let full_scrubbed =
            redact_secrets_multi_with_limit(&payload, std::slice::from_ref(&key), payload.len());
        assert!(!full_scrubbed.contains("0xsecret_preimage_val"));
        assert!(!full_scrubbed.contains("GameSaltKeep"));
        assert!(!full_scrubbed.contains("0x8000000000000001"));

        // Empty, already-redacted credential fields, and game-only fields produce zero paths.
        let clean_only = format!(
            r#"{{"salt":"keep","preimage":"0xabc","authorization":"","api_key":"{REDACTED_MARKER}"}}"#
        );
        let (clean_scrubbed, clean_paths) =
            redact_secrets_multi_with_limit_and_paths(&clean_only, &[], clean_only.len());
        assert_eq!(clean_scrubbed, clean_only);
        assert!(clean_paths.is_empty());

        // Malformed / truncated JSON falls back to Branch-1 blanked credential key paths only.
        let truncated_cred = r#"{"outer":{"secret_seed":"unterminated_cred_val"#;
        let (trunc_cred_scrubbed, trunc_cred_paths) =
            redact_secrets_multi_with_limit_and_paths(truncated_cred, &[], truncated_cred.len());
        assert!(!trunc_cred_scrubbed.contains("unterminated_cred_val"));
        assert_eq!(trunc_cred_paths, vec!["/secret_seed"]);

        let truncated_game = r#"{"outer":{"reveal_salt":"unterminated_game_val"#;
        let (trunc_game_scrubbed, trunc_game_paths) =
            redact_secrets_multi_with_limit_and_paths(truncated_game, &[], truncated_game.len());
        assert_eq!(trunc_game_scrubbed, truncated_game);
        assert!(trunc_game_paths.is_empty());
    }
}

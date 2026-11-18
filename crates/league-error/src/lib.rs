//! Shared canonical [`StructuredError`], [`ToStructuredError`], and secret scrubber
//! (`SHIP_GATE.md` `:26`) for Merchant League crates.
//!
//! Kept as a leaf crate depending only on `serde` and `serde_json` so every workspace crate
//! (`league-core`, `league-ledger`, `league-relay`, `league-standalone`, and `league-gen` via
//! `league-core`) shares one concrete error representation and scrubber without introducing a
//! `league-core` dependency in `league-relay` or `league-ledger`.

use std::fmt;

use serde::ser::SerializeStruct as _;
use serde::{Serialize, Serializer};

/// Default redaction marker substituted by [`scrub_secret_text`] and [`StructuredError`].
pub const REDACTED_PLACEHOLDER: &str = "[REDACTED]";

const RELAY_REDACTED_MARKER: &str = "<REDACTED_API_KEY>";

const SECRET_ENV_PREFIXES: [&str; 7] = [
    "OPENROUTER_API_KEY=",
    "ANTHROPIC_API_KEY=",
    "OPENAI_API_KEY=",
    "GEMINI_API_KEY=",
    "LEAGUE_API_KEY=",
    "VLLM_API_KEY=",
    "LOCAL_API_KEY=",
];

/// Returns `true` if `b` is in the XRPL Base58 alphabet.
#[must_use]
pub const fn is_base58_byte(b: u8) -> bool {
    matches!(
        b,
        b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'
    )
}

const fn is_token_stopper(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b',' | b';' | b')' | b']' | b'}' | b'"' | b'\'' | b'\\')
}

fn match_already_redacted<'a>(rest: &str, marker: &'a str) -> Option<&'a str> {
    for candidate in [marker, REDACTED_PLACEHOLDER, RELAY_REDACTED_MARKER] {
        if let Some(after) = rest.strip_prefix(candidate)
            && after
                .as_bytes()
                .first()
                .is_none_or(|b| is_token_stopper(*b))
        {
            return Some(candidate);
        }
    }
    None
}

/// Returns `true` if a Base58 byte slice looks like an XRPL family seed (`sEd...`, `sn...`,
/// or secp256k1 `s...` of `25..=35` Base58 bytes carrying mixed uppercase and digits).
#[must_use]
pub fn looks_like_xrpl_family_seed(slice: &[u8]) -> bool {
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

/// Redacts bearer tokens (`Bearer <token>` / `Bearer\t<token>`), prefixed API keys (`sk-or-`,
/// `sk-proj-`, `sk-ant-`), known secret env-var assignments (case-insensitive), and XRPL family
/// seeds using `marker` as the replacement placeholder.
#[must_use]
pub fn scrub_secret_text_with_marker(input: &str, marker: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < bytes.len() {
        let at_boundary = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();

        // 1. Case-insensitive `Bearer <token>` or `Bearer\t<token>` (C1) at a word boundary.
        if at_boundary
            && i + 7 <= bytes.len()
            && bytes[i..i + 6].eq_ignore_ascii_case(b"bearer")
            && matches!(bytes[i + 6], b' ' | b'\t')
        {
            out.push_str(&input[i..i + 6]);
            i += 6;
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                out.push(bytes[i] as char);
                i += 1;
            }
            if let Some(existing) = match_already_redacted(&input[i..], marker) {
                out.push_str(existing);
                i += existing.len();
                continue;
            }
            let start = i;
            while i < bytes.len() && !is_token_stopper(bytes[i]) {
                i += 1;
            }
            if i > start {
                out.push_str(marker);
            }
            continue;
        }

        // 2. Known secret env-var assignments (`KEY=<value>`, case-insensitive).
        let mut matched_len = None;
        if at_boundary {
            for prefix in SECRET_ENV_PREFIXES {
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
            if let Some(existing) = match_already_redacted(&input[i..], marker) {
                out.push_str(existing);
                i += existing.len();
                continue;
            }
            let start = i;
            while i < bytes.len() && !is_token_stopper(bytes[i]) {
                i += 1;
            }
            if i > start {
                out.push_str(marker);
            }
            continue;
        }

        // 3. API key tokens (`sk-or-...`, `sk-proj-...`, `sk-ant-...`).
        if at_boundary {
            let token_prefix_len = ["sk-or-", "sk-proj-", "sk-ant-"]
                .into_iter()
                .find(|p| input[i..].starts_with(p))
                .map(str::len);
            if let Some(plen) = token_prefix_len {
                let mut end = i + plen;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'-' | b'_'))
                {
                    end += 1;
                }
                if end > i + plen {
                    out.push_str(marker);
                    i = end;
                    continue;
                }
            }
        }

        // 4. XRPL base58 family seeds (`sEd...`, `sn...`, or `sp...`/`s...` 25..=35 base58).
        if at_boundary && bytes[i] == b's' {
            let mut end = i;
            while end < bytes.len() && is_base58_byte(bytes[end]) {
                end += 1;
            }
            let boundary_after = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
            if boundary_after && looks_like_xrpl_family_seed(&bytes[i..end]) {
                out.push_str(marker);
                i = end;
                continue;
            }
        }

        // Advance one UTF-8 scalar safely.
        let ch = input[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

/// Redacts bearer tokens, API keys, known secret env-var assignments, and XRPL seeds from
/// diagnostic or error text before printing or serializing.
#[must_use]
pub fn scrub_secret_text(input: &str) -> String {
    scrub_secret_text_with_marker(input, REDACTED_PLACEHOLDER)
}

/// Canonical machine- and human-readable error shape required across workspace crates
/// (`SHIP_GATE.md` `:26`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredError {
    /// Stable `SCREAMING_SNAKE_CASE` error identifier.
    pub code: &'static str,
    /// Scrubbed human-readable description of the failure.
    pub message: String,
    /// Actionable recovery hint for callers or operators.
    pub hint: &'static str,
    /// Optional scrubbed underlying cause chain.
    pub cause: Option<String>,
    /// Whether retrying the identical operation without changing configuration might succeed.
    pub retryable: bool,
}

impl Serialize for StructuredError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let scrubbed_message = scrub_secret_text(&self.message);
        let scrubbed_cause = self.cause.as_deref().map(scrub_secret_text);
        let field_count = if scrubbed_cause.is_some() { 5 } else { 4 };
        let mut state = serializer.serialize_struct("StructuredError", field_count)?;
        state.serialize_field("code", self.code)?;
        state.serialize_field("message", &scrubbed_message)?;
        state.serialize_field("hint", self.hint)?;
        if let Some(cause) = &scrubbed_cause {
            state.serialize_field("cause", cause)?;
        }
        state.serialize_field("retryable", &self.retryable)?;
        state.end()
    }
}

impl StructuredError {
    /// Constructs a [`StructuredError`] while scrubbing `message` of any accidental secret tokens.
    #[must_use]
    pub fn new(
        code: &'static str,
        message: impl Into<String>,
        hint: &'static str,
        retryable: bool,
    ) -> Self {
        Self {
            code,
            message: scrub_secret_text(&message.into()),
            hint,
            cause: None,
            retryable,
        }
    }

    /// Attaches a scrubbed underlying cause string to this [`StructuredError`].
    #[must_use]
    pub fn with_cause(mut self, cause: impl Into<String>) -> Self {
        self.cause = Some(scrub_secret_text(&cause.into()));
        self
    }

    /// Formats this error for single- or multi-line plain-text CLI/log output with scrubbed text.
    #[must_use]
    pub fn format_text(&self) -> String {
        let msg = scrub_secret_text(&self.message);
        let mut out = format!("[{}] {}", self.code, msg);
        if let Some(cause) = &self.cause {
            out.push_str(&format!(" (cause: {})", scrub_secret_text(cause)));
        }
        out.push_str(&format!("; hint: {}", self.hint));
        if self.retryable {
            out.push_str(" [retryable]");
        }
        out
    }

    /// Serializes this error as compact scrubbed JSON.
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

impl fmt::Display for StructuredError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format_text())
    }
}

impl std::error::Error for StructuredError {}

/// Conversion trait projecting crate error enums onto [`StructuredError`].
pub trait ToStructuredError {
    /// Projects `&self` onto a scrubbed [`StructuredError`].
    fn to_structured_error(&self) -> StructuredError;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_pre_change_league_core_7fd3b481_json_and_format_text_literals() {
        // 1. cause: None, retryable: false
        let c1 = StructuredError::new(
            "INVALID_CONFIG",
            "bad config OPENROUTER_API_KEY=sk-or-secret123",
            "check TableSpec",
            false,
        );
        assert_eq!(
            c1.to_json_string()
                .unwrap_or_else(|e| panic!("c1 json failed: {e}")),
            "{\"code\":\"INVALID_CONFIG\",\"message\":\"bad config OPENROUTER_API_KEY=[REDACTED]\",\"hint\":\"check TableSpec\",\"retryable\":false}"
        );
        assert_eq!(
            c1.format_text(),
            "[INVALID_CONFIG] bad config OPENROUTER_API_KEY=[REDACTED]; hint: check TableSpec"
        );
        assert_eq!(c1.to_string(), c1.format_text());

        // 2. cause: None, retryable: true
        let c2 = StructuredError::new(
            "TURN_LOCKED",
            "lock busy sk-or-v1-abc123",
            "wait and retry",
            true,
        );
        assert_eq!(
            c2.to_json_string()
                .unwrap_or_else(|e| panic!("c2 json failed: {e}")),
            "{\"code\":\"TURN_LOCKED\",\"message\":\"lock busy [REDACTED]\",\"hint\":\"wait and retry\",\"retryable\":true}"
        );
        assert_eq!(
            c2.format_text(),
            "[TURN_LOCKED] lock busy [REDACTED]; hint: wait and retry [retryable]"
        );

        // 3. cause: Some, retryable: false
        let c3 = StructuredError::new("RELAY_IO", "io error", "check path", false)
            .with_cause("GEMINI_API_KEY=AIzaSySecret12345 failed");
        assert_eq!(
            c3.to_json_string()
                .unwrap_or_else(|e| panic!("c3 json failed: {e}")),
            "{\"code\":\"RELAY_IO\",\"message\":\"io error\",\"hint\":\"check path\",\"cause\":\"GEMINI_API_KEY=[REDACTED] failed\",\"retryable\":false}"
        );
        assert_eq!(
            c3.format_text(),
            "[RELAY_IO] io error (cause: GEMINI_API_KEY=[REDACTED] failed); hint: check path"
        );

        // 4. cause: Some, retryable: true
        let c4 = StructuredError::new(
            "SIGN_ERR",
            "failed signing with sEd7f3A9kL2mN4pQ6rT8vW1xY3zB5cD",
            "rotate seed",
            true,
        )
        .with_cause("GEMINI_API_KEY=AIzaSySecret12345 caused retryable failure");
        assert_eq!(
            c4.to_json_string()
                .unwrap_or_else(|e| panic!("c4 json failed: {e}")),
            "{\"code\":\"SIGN_ERR\",\"message\":\"failed signing with [REDACTED]\",\"hint\":\"rotate seed\",\"cause\":\"GEMINI_API_KEY=[REDACTED] caused retryable failure\",\"retryable\":true}"
        );
        assert_eq!(
            c4.format_text(),
            "[SIGN_ERR] failed signing with [REDACTED] (cause: GEMINI_API_KEY=[REDACTED] caused retryable failure); hint: rotate seed [retryable]"
        );
    }

    #[test]
    fn b5_and_c1_planted_secrets_and_idempotency_across_shared_scrubber() {
        let planted_bearer_space = "Bearer tok_space_12345";
        let planted_bearer_tab = "Bearer\ttok_tab_67890";
        let planted_sk_or = "sk-or-v1-planted001";
        let planted_sk_proj = "sk-proj-planted002";
        let planted_sk_ant = "sk-ant-api03-planted003";
        let planted_ed = "sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r";
        let planted_secp = "sp5fghtJtpUorTwvof1NpDXAzNwf5";
        let planted_sn = "sn3nAkU7v8w9xPy1z2A3B4C5D6E7F";

        let raw = format!(
            "OPENROUTER_API_KEY=or_up openrouter_api_key=or_lo \
             ANTHROPIC_API_KEY=ant_up anthropic_api_key=ant_lo \
             OPENAI_API_KEY=oai_up openai_api_key=oai_lo \
             GEMINI_API_KEY=gem_up gemini_api_key=gem_lo \
             LEAGUE_API_KEY=lg_up league_api_key=lg_lo \
             VLLM_API_KEY=vl_up vllm_api_key=vl_lo \
             LOCAL_API_KEY=loc_up local_api_key=loc_lo \
             {planted_bearer_space} {planted_bearer_tab} \
             {planted_sk_or} {planted_sk_proj} {planted_sk_ant} \
             {planted_ed} {planted_secp} {planted_sn} \
             \\\"\
             GEMINI_API_KEY=escaped_gem\\\" \
             bearish market sn123 session_identifier_long_word_ok"
        );

        let scrubbed = scrub_secret_text(&raw);
        for needle in [
            "or_up",
            "or_lo",
            "ant_up",
            "ant_lo",
            "oai_up",
            "oai_lo",
            "gem_up",
            "gem_lo",
            "lg_up",
            "lg_lo",
            "vl_up",
            "vl_lo",
            "loc_up",
            "loc_lo",
            "tok_space_12345",
            "tok_tab_67890",
            planted_sk_or,
            planted_sk_proj,
            planted_sk_ant,
            planted_ed,
            planted_secp,
            planted_sn,
            "escaped_gem",
        ] {
            assert!(
                !scrubbed.contains(needle),
                "shared scrubber leaked '{needle}' in: {scrubbed}"
            );
        }
        assert!(scrubbed.contains("bearish market sn123 session_identifier_long_word_ok"));
        assert!(scrubbed.contains("\\\"GEMINI_API_KEY=[REDACTED]\\\""));
        assert_eq!(
            scrub_secret_text("Authorization: Bearer\tsecret_tab_val ok"),
            "Authorization: Bearer\t[REDACTED] ok"
        );
        assert_eq!(scrub_secret_text(&scrubbed), scrubbed);

        // Relay marker preservation & idempotency.
        let relay_scrubbed = scrub_secret_text_with_marker(&raw, RELAY_REDACTED_MARKER);
        assert!(relay_scrubbed.contains("Bearer\t<REDACTED_API_KEY>"));
        assert_eq!(scrub_secret_text(&relay_scrubbed), relay_scrubbed);
    }
}

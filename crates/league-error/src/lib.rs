#![no_std]
#![forbid(unsafe_code)]
#![forbid(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
//! Shared canonical [`StructuredError`], [`ToStructuredError`], and secret scrubber
//! (`SHIP_GATE.md` `:26`).
//!
//! Kept as a `no_std` + `alloc` leaf crate depending only on `serde` and `serde_json` so every
//! workspace crate (`league-core`, `league-ledger`, `league-relay`, `league-standalone`,
//! `league-seat-types`, and `league-gen` via `league-core`) shares one concrete error
//! representation and scrubber without introducing a `league-core` dependency in seat crates,
//! `league-relay`, or `league-ledger`.

extern crate alloc;

use alloc::format;
use alloc::string::String;
use core::fmt;

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

/// Returns `true` if byte offset `i` is at a word boundary, including immediately after a
/// preceding JSON escape (`\n`, `\t`, `\r`, `\b`, `\f`, `\"`, `\/`, `\uXXXX`) or URL
/// percent-encoded byte (`%xx`).
#[must_use]
pub const fn is_at_boundary(bytes: &[u8], i: usize) -> bool {
    if i == 0 || !bytes[i - 1].is_ascii_alphanumeric() {
        return true;
    }
    if i >= 2
        && bytes[i - 2] == b'\\'
        && matches!(bytes[i - 1], b'n' | b't' | b'r' | b'b' | b'f' | b'"' | b'/')
    {
        return true;
    }
    if i >= 6
        && bytes[i - 6] == b'\\'
        && bytes[i - 5] == b'u'
        && bytes[i - 4].is_ascii_hexdigit()
        && bytes[i - 3].is_ascii_hexdigit()
        && bytes[i - 2].is_ascii_hexdigit()
        && bytes[i - 1].is_ascii_hexdigit()
    {
        return true;
    }
    i >= 3
        && bytes[i - 3] == b'%'
        && bytes[i - 2].is_ascii_hexdigit()
        && bytes[i - 1].is_ascii_hexdigit()
}

/// Returns `true` if byte offset `idx` begins a token delimiter: ASCII whitespace, closing
/// punctuation, a bare quote, or a backslash run escaping a quote (`\"`, `\'`, `\\\"`).
///
/// A backslash followed by any non-quote byte (such as `\a` or `\d` inside `Bearer tok\abc`
/// or `OPENROUTER_API_KEY=abc\def`) is not a stopper, so the full token is scrubbed while
/// escaped JSON (`\"KEY=val\"`) stops cleanly before the closing quote escape.
#[must_use]
pub fn is_token_stopper_at(bytes: &[u8], idx: usize) -> bool {
    let Some(&b) = bytes.get(idx) else {
        return true;
    };
    if b.is_ascii_whitespace() || matches!(b, b',' | b';' | b')' | b']' | b'}' | b'"' | b'\'') {
        return true;
    }
    if b == b'\\' {
        let mut k = idx + 1;
        while k < bytes.len() && bytes[k] == b'\\' {
            k += 1;
        }
        return matches!(bytes.get(k), Some(b'"' | b'\''));
    }
    false
}

fn match_already_redacted<'a>(rest: &str, marker: &'a str) -> Option<&'a str> {
    for candidate in [marker, REDACTED_PLACEHOLDER, RELAY_REDACTED_MARKER] {
        if let Some(after) = rest.strip_prefix(candidate)
            && is_token_stopper_at(after.as_bytes(), 0)
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
        let at_boundary = is_at_boundary(bytes, i);

        // 1. Case-insensitive `Bearer <token>` or `Bearer\t<token>` anywhere (`9818e991` relay + C1).
        if i + 7 <= bytes.len()
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
            while i < bytes.len() && !is_token_stopper_at(bytes, i) {
                i += 1;
            }
            if i > start {
                out.push_str(marker);
            }
            continue;
        }

        // 2. Known secret env-var assignments (`KEY=<value>`, case-insensitive, at word/escape boundary).
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
            while i < bytes.len() && !is_token_stopper_at(bytes, i) {
                i += 1;
            }
            if i > start {
                out.push_str(marker);
            }
            continue;
        }

        // 3. API key tokens (`sk-or-...`, `sk-proj-...`, `sk-ant-...`), case-insensitive anywhere.
        let token_prefix_len = [
            b"sk-or-".as_slice(),
            b"sk-proj-".as_slice(),
            b"sk-ant-".as_slice(),
        ]
        .into_iter()
        .find(|p| bytes.len() - i >= p.len() && bytes[i..i + p.len()].eq_ignore_ascii_case(p))
        .map(<[u8]>::len);
        if let Some(plen) = token_prefix_len {
            let mut end = i + plen;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'-' | b'_' | b'.'))
            {
                end += 1;
            }
            if end > i + plen {
                out.push_str(marker);
                i = end;
                continue;
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
#[derive(Clone, PartialEq, Eq)]
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

impl fmt::Debug for StructuredError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scrubbed_message = scrub_secret_text(&self.message);
        let scrubbed_cause = self.cause.as_deref().map(scrub_secret_text);
        f.debug_struct("StructuredError")
            .field("code", &self.code)
            .field("message", &scrubbed_message)
            .field("hint", &self.hint)
            .field("cause", &scrubbed_cause)
            .field("retryable", &self.retryable)
            .finish()
    }
}

impl Serialize for StructuredError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let scrubbed_message = scrub_secret_text(&self.message);
        let scrubbed_cause = self.cause.as_deref().map(scrub_secret_text);
        let mut state = serializer.serialize_struct("StructuredError", 5)?;
        state.serialize_field("code", self.code)?;
        state.serialize_field("message", &scrubbed_message)?;
        state.serialize_field("hint", self.hint)?;
        if let Some(cause) = &scrubbed_cause {
            state.serialize_field("cause", cause)?;
        } else {
            state.skip_field("cause")?;
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

impl core::error::Error for StructuredError {}

/// Conversion trait projecting crate error enums onto [`StructuredError`].
pub trait ToStructuredError {
    /// Projects `&self` onto a scrubbed [`StructuredError`].
    fn to_structured_error(&self) -> StructuredError;
}

/// Temporary negative-control probe with untested lines to prove `codecov/patch`
/// rejects diffs below 90%; reverted immediately after CI confirms failure.
#[doc(hidden)]
#[must_use]
pub fn _planted_patch_coverage_probe(selector: u32) -> u32 {
    let base = selector.wrapping_add(3);
    if base < 10 {
        return base.saturating_mul(2);
    }
    if base < 20 {
        return base.saturating_add(5);
    }
    if base < 30 {
        return base.saturating_sub(4);
    }
    if base < 40 {
        return base ^ 0x5A;
    }
    base.wrapping_mul(7)
}

#[cfg(test)]
mod tests {
    use alloc::borrow::ToOwned;
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn secret_free_shapes_match_captured_7fd3b481_league_core_bytes() {
        // Exact JSON and `format_text()` bytes captured from `league-core` at `7fd3b481`.
        // 1. cause: None, retryable: false
        let p1 = StructuredError::new(
            "INVALID_CONFIG",
            "bad config field table_mode mismatch",
            "check TableSpec",
            false,
        );
        assert_eq!(
            p1.to_json_string()
                .unwrap_or_else(|e| panic!("p1 json failed: {e}")),
            "{\"code\":\"INVALID_CONFIG\",\"message\":\"bad config field table_mode mismatch\",\"hint\":\"check TableSpec\",\"retryable\":false}"
        );
        assert_eq!(
            p1.format_text(),
            "[INVALID_CONFIG] bad config field table_mode mismatch; hint: check TableSpec"
        );
        assert_eq!(p1.to_string(), p1.format_text());

        // 2. cause: None, retryable: true
        let p2 = StructuredError::new(
            "TURN_LOCKED",
            "turn phase lock active for seat 2",
            "wait and retry",
            true,
        );
        assert_eq!(
            p2.to_json_string()
                .unwrap_or_else(|e| panic!("p2 json failed: {e}")),
            "{\"code\":\"TURN_LOCKED\",\"message\":\"turn phase lock active for seat 2\",\"hint\":\"wait and retry\",\"retryable\":true}"
        );
        assert_eq!(
            p2.format_text(),
            "[TURN_LOCKED] turn phase lock active for seat 2; hint: wait and retry [retryable]"
        );

        // 3. cause: Some, retryable: false
        let p3 = StructuredError::new("RELAY_IO", "io error reading state", "check path", false)
            .with_cause("underlying file not found");
        assert_eq!(
            p3.to_json_string()
                .unwrap_or_else(|e| panic!("p3 json failed: {e}")),
            "{\"code\":\"RELAY_IO\",\"message\":\"io error reading state\",\"hint\":\"check path\",\"cause\":\"underlying file not found\",\"retryable\":false}"
        );
        assert_eq!(
            p3.format_text(),
            "[RELAY_IO] io error reading state (cause: underlying file not found); hint: check path"
        );

        // 4. cause: Some, retryable: true
        let p4 = StructuredError::new("SIGN_ERR", "failed signing tx", "rotate seed", true)
            .with_cause("connection timed out caused retryable failure");
        assert_eq!(
            p4.to_json_string()
                .unwrap_or_else(|e| panic!("p4 json failed: {e}")),
            "{\"code\":\"SIGN_ERR\",\"message\":\"failed signing tx\",\"hint\":\"rotate seed\",\"cause\":\"connection timed out caused retryable failure\",\"retryable\":true}"
        );
        assert_eq!(
            p4.format_text(),
            "[SIGN_ERR] failed signing tx (cause: connection timed out caused retryable failure); hint: rotate seed [retryable]"
        );
    }

    #[test]
    fn secret_bearing_structured_error_scrubs_once_without_double_bracket_bug() {
        // New fixed behaviour (differs from 7fd3b481 which double-scrubbed `[REDACTED]` into
        // `[REDACTED]]` and left literal `serde_json::to_string` unscrubbed).
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

        // Field-by-field struct construction still scrubs during `serde_json::to_string`.
        let raw_literal = StructuredError {
            code: "RAW",
            message: "SK-OR-V1-RAW123".to_owned(),
            hint: "hint",
            cause: Some("OPENROUTER_API_KEY=abc\\def".to_owned()),
            retryable: false,
        };
        let literal_json = serde_json::to_string(&raw_literal)
            .unwrap_or_else(|e| panic!("raw literal json failed: {e}"));
        assert!(literal_json.contains("\"message\":\"[REDACTED]\""));
        assert!(literal_json.contains("\"cause\":\"OPENROUTER_API_KEY=[REDACTED]\""));
    }

    #[test]
    fn planted_union_rules_escape_boundaries_backslash_tokens_and_idempotency() {
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

        // C1: Backslash inside Bearer / env tokens redacts through non-quote escapes,
        // while `\"` still stops cleanly for escaped JSON.
        assert_eq!(
            scrub_secret_text("Bearer tok\\abc end"),
            "Bearer [REDACTED] end"
        );
        assert_eq!(
            scrub_secret_text("OPENROUTER_API_KEY=abc\\def"),
            "OPENROUTER_API_KEY=[REDACTED]"
        );

        // A1 & B3: Anywhere + case-insensitive prefixes and escape boundaries.
        assert_eq!(
            scrub_secret_text("body \\nsk-or-v1-leakEscNewline end"),
            "body \\n[REDACTED] end"
        );
        assert_eq!(scrub_secret_text("SK-OR-V1-LEAKUPPER123"), "[REDACTED]");
        assert_eq!(scrub_secret_text("keysk-proj-leakGlued1"), "key[REDACTED]");
        assert_eq!(
            scrub_secret_text("xBearer leakBearerGlued"),
            "xBearer [REDACTED]"
        );
        assert_eq!(
            scrub_secret_text("\\nsEdSKaCy2JT7JaM7v95H9SxkhP9wS2r"),
            "\\n[REDACTED]"
        );
        assert_eq!(
            scrub_secret_text("\\nOPENROUTER_API_KEY=leakEnvNL"),
            "\\nOPENROUTER_API_KEY=[REDACTED]"
        );

        // PR #80 C1: `\uXXXX`, `\b`, and `\f` escapes act as word boundaries too.
        assert_eq!(
            scrub_secret_text("\\u000aOPENROUTER_API_KEY=leakUnicodeEsc1"),
            "\\u000aOPENROUTER_API_KEY=[REDACTED]"
        );
        assert_eq!(
            scrub_secret_text("\\u000asEdSKaCy2JT7JaM7v95H9SxkhP9wS2r"),
            "\\u000a[REDACTED]"
        );
        assert_eq!(
            scrub_secret_text("\\u001Bsp5fghtJtpUorTwvof1NpDXAzNwf5"),
            "\\u001B[REDACTED]"
        );
        assert_eq!(
            scrub_secret_text("\\bOPENROUTER_API_KEY=leakBackspaceEsc1"),
            "\\bOPENROUTER_API_KEY=[REDACTED]"
        );
        assert_eq!(
            scrub_secret_text("\\bsn3nAkU7v8w9xPy1z2A3B4C5D6E7F"),
            "\\b[REDACTED]"
        );
        assert_eq!(
            scrub_secret_text("\\fsEdSKaCy2JT7JaM7v95H9SxkhP9wS2r"),
            "\\f[REDACTED]"
        );
        assert_eq!(
            scrub_secret_text("\\fANTHROPIC_API_KEY=leakFormfeedEsc1"),
            "\\fANTHROPIC_API_KEY=[REDACTED]"
        );

        assert_eq!(scrub_secret_text(&scrubbed), scrubbed);

        // Relay marker preservation & idempotency.
        let relay_scrubbed = scrub_secret_text_with_marker(&raw, RELAY_REDACTED_MARKER);
        assert!(relay_scrubbed.contains("Bearer\t<REDACTED_API_KEY>"));
        assert_eq!(scrub_secret_text(&relay_scrubbed), relay_scrubbed);
    }

    #[test]
    fn structured_error_debug_scrubs_raw_message_and_cause_like_display() {
        let raw_literal = StructuredError {
            code: "RAW_DEBUG",
            message: "seed sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r key sk-proj-secretDebug123".to_owned(),
            hint: "check inputs",
            cause: Some("Bearer tok_debug_secret OPENROUTER_API_KEY=or_debug_val".to_owned()),
            retryable: false,
        };
        let dbg = format!("{raw_literal:?}");
        let disp = format!("{raw_literal}");
        for secret in [
            "sEdSKaCy2JT7JaM7v95H9SxkhP9wS2r",
            "sk-proj-secretDebug123",
            "tok_debug_secret",
            "or_debug_val",
        ] {
            assert!(!dbg.contains(secret), "Debug leaked `{secret}`: {dbg}");
            assert!(!disp.contains(secret), "Display leaked `{secret}`: {disp}");
        }
        assert!(dbg.contains("[REDACTED]"));
        assert!(dbg.contains("RAW_DEBUG"));

        let clean = StructuredError::new("CLEAN_CODE", "normal text", "normal hint", true)
            .with_cause("plain cause");
        assert_eq!(
            format!("{clean:?}"),
            "StructuredError { code: \"CLEAN_CODE\", message: \"normal text\", hint: \"normal hint\", cause: Some(\"plain cause\"), retryable: true }"
        );
    }
}

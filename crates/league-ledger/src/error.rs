//! Structured operator/caller error representation (`SHIP_GATE.md` `:26`) with built-in secret
//! scrubbing before text or JSON emission.

use std::fmt;

use serde::ser::SerializeStruct as _;
use serde::{Serialize, Serializer};

const REDACTED: &str = "[REDACTED]";

const SECRET_ENV_PREFIXES: [&str; 7] = [
    "OPENROUTER_API_KEY=",
    "ANTHROPIC_API_KEY=",
    "OPENAI_API_KEY=",
    "GEMINI_API_KEY=",
    "LEAGUE_API_KEY=",
    "VLLM_API_KEY=",
    "LOCAL_API_KEY=",
];

const fn is_base58_char(b: u8) -> bool {
    matches!(
        b,
        b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'
    )
}

const fn is_token_stopper(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b',' | b';' | b')' | b']' | b'}' | b'"' | b'\'')
}

fn is_already_redacted(rest: &str) -> bool {
    let Some(after) = rest.strip_prefix(REDACTED) else {
        return false;
    };
    after
        .as_bytes()
        .first()
        .is_none_or(|b| is_token_stopper(*b))
}

fn looks_like_xrpl_family_seed(slice: &[u8]) -> bool {
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

/// Redacts bearer tokens, API keys, known secret env-var assignments, and XRPL seeds from
/// diagnostic or error text before printing or serializing.
#[must_use]
pub fn scrub_secret_text(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < bytes.len() {
        // 1. Case-insensitive `Bearer <token>` (only when `bearer ` starts at a word boundary).
        let at_boundary = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if at_boundary && i + 7 <= bytes.len() && bytes[i..i + 7].eq_ignore_ascii_case(b"bearer ") {
            out.push_str(&input[i..i + 7]);
            i += 7;
            while i < bytes.len() && bytes[i] == b' ' {
                out.push(' ');
                i += 1;
            }
            if is_already_redacted(&input[i..]) {
                out.push_str(REDACTED);
                i += REDACTED.len();
                continue;
            }
            let start = i;
            while i < bytes.len() && !is_token_stopper(bytes[i]) {
                i += 1;
            }
            if i > start {
                out.push_str(REDACTED);
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
            if is_already_redacted(&input[i..]) {
                out.push_str(REDACTED);
                i += REDACTED.len();
                continue;
            }
            let start = i;
            while i < bytes.len() && !is_token_stopper(bytes[i]) {
                i += 1;
            }
            if i > start {
                out.push_str(REDACTED);
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
                    out.push_str(REDACTED);
                    i = end;
                    continue;
                }
            }
        }

        // 4. XRPL base58 family seeds (`sEd...`, `sn...`, or `sp...`/`s...` 25..=35 base58).
        if at_boundary && bytes[i] == b's' {
            let mut end = i;
            while end < bytes.len() && is_base58_char(bytes[end]) {
                end += 1;
            }
            let boundary_after = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
            if boundary_after && looks_like_xrpl_family_seed(&bytes[i..end]) {
                out.push_str(REDACTED);
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

//! Structured operator/caller error representation (`SHIP_GATE.md` `:26`) with built-in secret
//! scrubbing before text or JSON emission.

use std::fmt;

use serde::Serialize;

use crate::replay::ReplayLayoutSeedError;
use crate::types::EngineError;

const REDACTED: &str = "[REDACTED]";

const SECRET_ENV_PREFIXES: [&str; 5] = [
    "OPENROUTER_API_KEY=",
    "ANTHROPIC_API_KEY=",
    "OPENAI_API_KEY=",
    "GEMINI_API_KEY=",
    "LEAGUE_API_KEY=",
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
            let start = i;
            while i < bytes.len() && !is_token_stopper(bytes[i]) {
                i += 1;
            }
            if i > start {
                out.push_str(REDACTED);
            }
            continue;
        }

        // 2. Known secret env-var assignments (`KEY=<value>`).
        let mut matched_env = None;
        if at_boundary {
            for prefix in SECRET_ENV_PREFIXES {
                if input[i..].starts_with(prefix) {
                    matched_env = Some(prefix);
                    break;
                }
            }
        }
        if let Some(prefix) = matched_env {
            out.push_str(prefix);
            i += prefix.len();
            let start = i;
            while i < bytes.len() && !is_token_stopper(bytes[i]) {
                i += 1;
            }
            if i > start {
                out.push_str(REDACTED);
            }
            continue;
        }

        // 3. OpenRouter API key tokens (`sk-or-...`).
        if at_boundary && input[i..].starts_with("sk-or-") {
            let mut end = i + 6;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'-' | b'_'))
            {
                end += 1;
            }
            if end > i + 6 {
                out.push_str(REDACTED);
                i = end;
                continue;
            }
        }

        // 4. XRPL base58 family seeds (`sEd...` / `sn...` of 22+ base58 characters).
        if at_boundary && (input[i..].starts_with("sEd") || input[i..].starts_with("sn")) {
            let mut end = i;
            while end < bytes.len() && is_base58_char(bytes[end]) {
                end += 1;
            }
            let boundary_after = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
            if end - i >= 22 && boundary_after {
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StructuredError {
    /// Stable `SCREAMING_SNAKE_CASE` error identifier.
    pub code: &'static str,
    /// Scrubbed human-readable description of the failure.
    pub message: String,
    /// Actionable recovery hint for callers or operators.
    pub hint: &'static str,
    /// Optional scrubbed underlying cause chain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// Whether retrying the identical operation without changing configuration might succeed.
    pub retryable: bool,
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
        let scrubbed = Self {
            code: self.code,
            message: scrub_secret_text(&self.message),
            hint: self.hint,
            cause: self.cause.as_deref().map(scrub_secret_text),
            retryable: self.retryable,
        };
        serde_json::to_string(&scrubbed)
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

impl ToStructuredError for EngineError {
    fn to_structured_error(&self) -> StructuredError {
        let hint = match self {
            Self::InvalidConfig(_) => {
                "check TableSpec, mode, seed pool, and replay fields against table validation rules"
            }
            Self::InvalidPhase(_) => {
                "submit the operation during the matching TurnPhase (TalkAndMarket, Commit, Reveal, or AwaitingMandateReveal)"
            }
            Self::InvalidSeat(_) => "use a valid zero-based seat index in 0..seat_count",
            Self::InvalidOrder(_) => {
                "adjust quantities, deadlines, assets, or VRF/replay parameters to satisfy rule bounds"
            }
            Self::UnknownObject(_) => {
                "verify the offer, check, escrow, project, or cartel domain ID exists in current state"
            }
            Self::Unauthorized(_) => {
                "ensure the acting seat owns the object, holds the required charter, and is not wash-frozen"
            }
            Self::PerishableEscrowForbidden => {
                "lock CRED collateral rather than perishable raw commodities inside TokenEscrow"
            }
            Self::InsufficientBalance(_) => {
                "reduce the order size or acquire enough free CRED or commodity units first"
            }
            Self::AlreadySubmitted(_) => {
                "wait for the next turn or phase rather than submitting a duplicate commitment or reveal"
            }
            Self::CommitmentMismatch(_) => {
                "reveal the exact SealedOrderBundle or Mandate and RevealSalt committed earlier"
            }
            Self::DomainAccessDenied(_) => {
                "obtain cartel domain membership or an active ClearinghouseBroker charter before accessing this domain"
            }
            Self::CancelLocked => {
                "cancel resting offers before Phase-1 cancel lock engages; late cancellations are rejected"
            }
            Self::SeasonEnded(_) => {
                "verify the replay log contains a complete closed season from Turn 1 through AwaitingMandateReveal"
            }
            Self::UnsupportedReplayVersion { .. } => {
                "upgrade the reader to support this replay format_version before loading the file"
            }
        };
        StructuredError::new(self.code(), self.to_string(), hint, false)
    }
}

impl ToStructuredError for ReplayLayoutSeedError {
    fn to_structured_error(&self) -> StructuredError {
        let hint = match self {
            Self::SealedSeedInClear { .. } => {
                "redact sealed-range layout seeds via SeasonReplayLog::new_sealed or from_rig_seed"
            }
            Self::InvalidSealedPool { .. } => {
                "set sealed layout seed pool to 'sealed' with its 64-hex SHA-256 digest"
            }
            Self::InvalidSealedDigest => {
                "provide a 64-character lowercase ASCII hex SHA-256 digest for the sealed layout seed"
            }
            Self::NonSealedSeedInSealed { .. } => {
                "use ReplayLayoutSeed::try_clear or SeasonReplayLog::from_rig_seed for non-sealed seeds (< 2^63)"
            }
        };
        StructuredError::new(self.code(), self.to_string(), hint, false)
    }
}

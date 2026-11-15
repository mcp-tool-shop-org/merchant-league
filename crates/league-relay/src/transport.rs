//! HTTP transport abstraction, live `ureq` implementation, and directory mock transport.
//!
//! All relay logic runs against [`Transport`] so tests and offline mock runs
//! exercise every code path without touching a socket. [`UreqTransport`] can
//! only be constructed from a [`LiveGate`].

use crate::error::RelayError;
use crate::secret::{ApiKey, LiveGate, redact_secrets};
use serde::Deserialize;
use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Duration;
use ureq::Agent;

/// Default OpenRouter Chat Completions endpoint.
pub const OPENROUTER_CHAT_COMPLETIONS_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

/// Raw HTTP status and text body returned by a [`Transport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawHttpResponse {
    /// HTTP status code (for example `200`, `429`, `502`).
    pub status: u16,
    /// Raw UTF-8 response body.
    pub body: String,
}

/// Transport-layer failure prior to receiving a complete HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The seat's wall timeout expired before a complete response arrived.
    Timeout,
    /// Redacted connection, DNS, TLS, or body-read error.
    Network(String),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::Timeout => f.write_str("request timed out"),
            TransportError::Network(msg) => write!(f, "transport error: {msg}"),
        }
    }
}

impl std::error::Error for TransportError {}

/// Synchronous HTTP transport for chat-completions POST calls.
pub trait Transport: Send + Sync {
    /// Send `body` (JSON) to `url` bounded by wall timeout `timeout`.
    fn post_chat_completions(
        &self,
        url: &str,
        body: &str,
        timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError>;
}

/// Map a `ureq::Error` into a redacted [`TransportError`].
pub fn map_ureq_error(err: ureq::Error, key: &ApiKey) -> TransportError {
    match err {
        ureq::Error::Timeout(_) => TransportError::Timeout,
        other => TransportError::Network(redact_secrets(&other.to_string(), Some(key))),
    }
}

/// Live HTTPS transport backed by `ureq`.
///
/// Requires a [`LiveGate`] so it cannot be constructed unless `--live`,
/// `--cap-credits`, and `OPENROUTER_API_KEY` were all verified.
pub struct UreqTransport {
    gate: LiveGate,
}

impl UreqTransport {
    /// Build a live `ureq` transport from a verified [`LiveGate`].
    pub fn new(gate: LiveGate) -> Self {
        Self { gate }
    }
}

impl fmt::Debug for UreqTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UreqTransport")
            .field("gate", &self.gate)
            .finish()
    }
}

impl Transport for UreqTransport {
    fn post_chat_completions(
        &self,
        url: &str,
        body: &str,
        timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build()
            .into();
        let auth = format!("Bearer {}", self.gate.api_key().secret_str());
        let resp = agent
            .post(url)
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .send(body)
            .map_err(|e| map_ureq_error(e, self.gate.api_key()))?;
        let status = resp.status().as_u16();
        let text = resp
            .into_body()
            .read_to_string()
            .map_err(|e| map_ureq_error(e, self.gate.api_key()))?;
        Ok(RawHttpResponse { status, body: text })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct MockRuleWire {
    when_contains: String,
    #[serde(default = "default_ok_status")]
    status: u16,
    #[serde(default)]
    response: serde_json::Value,
    #[serde(default)]
    panic_message: Option<String>,
}

fn default_ok_status() -> u16 {
    200
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct MockFileWire {
    rules: Vec<MockRuleWire>,
}

#[derive(Debug, Clone)]
struct MockRule {
    needle: String,
    status: u16,
    resp_body: String,
    panic_message: Option<String>,
}

/// Offline file-backed mock transport for `league-relay turn --mock-dir <dir>`.
#[derive(Debug, Clone)]
pub struct MockDirTransport {
    rules: Vec<MockRule>,
}

impl MockDirTransport {
    /// Load canned mock responses from `<dir>/responses.json`.
    pub fn load(dir: &Path) -> Result<Self, RelayError> {
        Self::load_with_key(dir, None)
    }

    /// Load canned mock responses from `<dir>/responses.json`, redacting any error details.
    pub fn load_with_key(dir: &Path, redaction_key: Option<&ApiKey>) -> Result<Self, RelayError> {
        let path = dir.join("responses.json");
        let raw = fs::read_to_string(&path).map_err(|e| {
            RelayError::Io(redact_secrets(
                &format!("could not read {}: {e}", path.display()),
                redaction_key,
            ))
        })?;
        let parsed: MockFileWire = serde_json::from_str(&raw).map_err(|e| {
            RelayError::Io(redact_secrets(
                &format!("invalid mock JSON in {}: {e}", path.display()),
                redaction_key,
            ))
        })?;
        let rules = parsed
            .rules
            .into_iter()
            .map(|r| MockRule {
                needle: r.when_contains,
                status: r.status,
                resp_body: r.response.to_string(),
                panic_message: r.panic_message,
            })
            .collect();
        Ok(Self { rules })
    }
}

impl Transport for MockDirTransport {
    fn post_chat_completions(
        &self,
        _url: &str,
        body: &str,
        _timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        for rule in &self.rules {
            if body.contains(&rule.needle) {
                if let Some(panic_msg) = &rule.panic_message {
                    panic!("{panic_msg}");
                }
                return Ok(RawHttpResponse {
                    status: rule.status,
                    body: rule.resp_body.clone(),
                });
            }
        }
        Err(TransportError::Network(
            "no matching rule in mock responses.json".to_owned(),
        ))
    }
}

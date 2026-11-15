//! HTTP transport abstraction, OpenRouter `ureq` implementation, local
//! OpenAI-compatible VM implementation, and directory mock transport.
//!
//! All relay logic runs against [`Transport`] so tests and offline mock runs
//! exercise every code path. [`UreqTransport`] can only be constructed from a
//! [`LiveGate`], and [`LocalOpenAiTransport`] can only be constructed from a
//! [`LocalGate`] (which refuses any API key, enforces loopback / VM-local URLs,
//! disables HTTP redirects, ignores environment proxies, and never sends an
//! `Authorization` header).

use crate::error::RelayError;
use crate::seats::{validate_local_base_url, validate_local_tokenize_url};
use crate::secret::{ApiKey, LiveGate, LocalGate, redact_secrets};
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

/// Synchronous HTTP transport for chat-completions POST calls and local token counting.
pub trait Transport: Send + Sync {
    /// Send `body` (JSON) to `url` bounded by wall timeout `timeout`.
    fn post_chat_completions(
        &self,
        url: &str,
        body: &str,
        timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError>;

    /// Count model tokens in `text` via the local server's `POST /tokenize` endpoint (`A-3`,
    /// `<https://docs.vllm.ai/en/stable/serving/online_serving/openai_compatible_server/#tokenizer-api>`).
    ///
    /// Transports without a tokenizer endpoint return `Err` so the relay refuses the turn
    /// rather than guessing by word or character count.
    fn count_tokens(
        &self,
        tokenize_url: &str,
        model: &str,
        text: &str,
        timeout: Duration,
    ) -> Result<usize, TransportError> {
        let _ = (tokenize_url, model, text, timeout);
        Err(TransportError::Network(
            "token counting endpoint (/tokenize) is unavailable on this transport".to_owned(),
        ))
    }
}

/// Map a `ureq::Error` into a redacted [`TransportError`].
pub fn map_ureq_error(err: ureq::Error, key: &ApiKey) -> TransportError {
    match err {
        ureq::Error::Timeout(_) => TransportError::Timeout,
        other => TransportError::Network(redact_secrets(&other.to_string(), Some(key))),
    }
}

/// Map a keyless local `ureq::Error` into a redacted [`TransportError`].
pub fn map_local_ureq_error(err: ureq::Error) -> TransportError {
    match err {
        ureq::Error::Timeout(_) => TransportError::Timeout,
        other => TransportError::Network(redact_secrets(&other.to_string(), None)),
    }
}

/// Live OpenRouter HTTPS transport backed by `ureq`.
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

/// Keyless HTTP transport for OpenAI-compatible servers (vLLM or similar) on loopback / VM.
///
/// Requires a [`LocalGate`], sends no `Authorization` header, disables HTTP
/// redirects (`max_redirects(0)`), ignores environment proxies (`proxy(None)`),
/// and re-validates every target URL against loopback and the gate's allowed
/// VM-local hosts before connecting.
#[derive(Debug, Clone)]
pub struct LocalOpenAiTransport {
    gate: LocalGate,
}

impl LocalOpenAiTransport {
    /// Build a keyless local transport from a verified [`LocalGate`].
    pub fn new(gate: LocalGate) -> Self {
        Self { gate }
    }

    /// Underlying [`LocalGate`].
    pub fn gate(&self) -> &LocalGate {
        &self.gate
    }
}

impl Transport for LocalOpenAiTransport {
    fn post_chat_completions(
        &self,
        url: &str,
        body: &str,
        timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        let (validated_endpoint, _) =
            validate_local_base_url("local", url, self.gate.allowed_vm_hosts())
                .map_err(|e| TransportError::Network(e.to_string()))?;
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .max_redirects(0)
            .proxy(None)
            .build()
            .into();
        let resp = agent
            .post(&validated_endpoint)
            .header("Content-Type", "application/json")
            .send(body)
            .map_err(map_local_ureq_error)?;
        let status = resp.status().as_u16();
        let text = resp
            .into_body()
            .read_to_string()
            .map_err(map_local_ureq_error)?;
        Ok(RawHttpResponse { status, body: text })
    }

    fn count_tokens(
        &self,
        tokenize_url: &str,
        model: &str,
        text: &str,
        timeout: Duration,
    ) -> Result<usize, TransportError> {
        let (validated_url, _) =
            validate_local_tokenize_url("local", tokenize_url, self.gate.allowed_vm_hosts())
                .map_err(|e| TransportError::Network(e.to_string()))?;
        let payload = serde_json::json!({
            "model": model,
            "prompt": text,
            "add_special_tokens": false,
        })
        .to_string();
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .max_redirects(0)
            .proxy(None)
            .build()
            .into();
        let resp = agent
            .post(&validated_url)
            .header("Content-Type", "application/json")
            .send(&payload)
            .map_err(map_local_ureq_error)?;
        let status = resp.status().as_u16();
        let body = resp
            .into_body()
            .read_to_string()
            .map_err(map_local_ureq_error)?;
        if status != 200 {
            return Err(TransportError::Network(format!(
                "POST /tokenize returned HTTP {status}"
            )));
        }
        let parsed: serde_json::Value = serde_json::from_str(&body).map_err(|_| {
            TransportError::Network("POST /tokenize returned invalid JSON".to_owned())
        })?;
        let Some(raw_count) = parsed.get("count").and_then(serde_json::Value::as_u64) else {
            return Err(TransportError::Network(
                "POST /tokenize response missing non-negative integer 'count'".to_owned(),
            ));
        };
        usize::try_from(raw_count).map_err(|_| {
            TransportError::Network("POST /tokenize 'count' overflowed usize".to_owned())
        })
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
    #[cfg(any(test, feature = "test-hooks"))]
    #[serde(default)]
    panic_message: Option<String>,
    #[cfg(any(test, feature = "test-hooks"))]
    #[serde(default)]
    sleep_ms: Option<u64>,
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
    #[cfg(any(test, feature = "test-hooks"))]
    panic_message: Option<String>,
    #[cfg(any(test, feature = "test-hooks"))]
    sleep_ms: Option<u64>,
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
                #[cfg(any(test, feature = "test-hooks"))]
                panic_message: r.panic_message,
                #[cfg(any(test, feature = "test-hooks"))]
                sleep_ms: r.sleep_ms,
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
                #[cfg(any(test, feature = "test-hooks"))]
                if let Some(ms) = rule.sleep_ms {
                    std::thread::sleep(Duration::from_millis(ms));
                }
                #[cfg(any(test, feature = "test-hooks"))]
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

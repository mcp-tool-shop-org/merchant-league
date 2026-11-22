//! HTTP transport abstraction, OpenRouter `ureq` implementation, local
//! OpenAI-compatible VM implementation, and directory mock transport.
//!
//! All relay logic runs against [`Transport`] so tests and offline mock runs
//! exercise every code path. [`UreqTransport`] can only be constructed from a
//! [`LiveGate`], and [`LocalOpenAiTransport`] can only be constructed from a
//! [`LocalGate`] (which refuses any API key, enforces loopback / VM-local URLs,
//! disables HTTP redirects, ignores environment proxies, and never sends an
//! `Authorization` header).

use crate::error::{RelayError, StructuredError, ToStructuredError};
use crate::seats::{
    validate_local_apply_template_url, validate_local_base_url, validate_local_tokenize_url,
};
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

impl TransportError {
    /// Stable `SCREAMING_SNAKE_CASE` error identifier (`SHIP_GATE.md` `:26`).
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Timeout => "TRANSPORT_TIMEOUT",
            Self::Network(_) => "TRANSPORT_NETWORK",
        }
    }
}

impl ToStructuredError for TransportError {
    fn to_structured_error(&self) -> StructuredError {
        match self {
            Self::Timeout => StructuredError::new(
                self.code(),
                self.to_string(),
                "increase the seat's timeout_secs (and max_gpu_secs_per_call if local) or retry when the endpoint is responsive",
                true,
            ),
            Self::Network(msg) => StructuredError::new(
                self.code(),
                redact_secrets(&self.to_string(), None),
                "verify the local server or OpenRouter endpoint is reachable and returns valid HTTP responses",
                true,
            )
            .with_cause(redact_secrets(msg, None)),
        }
    }
}

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

    /// Render chat messages, tools, and template options via `POST /apply-template`
    /// (`tools/server/server.cpp:292`, `tools/server/server-context.cpp:5416-5426`).
    ///
    /// Transports without `/apply-template` support fail closed.
    fn apply_template(
        &self,
        apply_template_url: &str,
        body: &str,
        timeout: Duration,
    ) -> Result<String, TransportError> {
        let _ = (apply_template_url, body, timeout);
        Err(TransportError::Network(
            "apply-template endpoint (/apply-template) is unavailable on this transport".to_owned(),
        ))
    }

    /// Tokenize rendered text via `POST /tokenize` returning server token IDs
    /// (`tools/server/server.cpp:290`, `tools/server/server-context.cpp:5440-5479`).
    ///
    /// Transports without `/tokenize` token ID support fail closed.
    fn tokenize_prompt(
        &self,
        tokenize_url: &str,
        content: &str,
        add_special: bool,
        parse_special: bool,
        timeout: Duration,
    ) -> Result<Vec<u64>, TransportError> {
        let _ = (tokenize_url, content, add_special, parse_special, timeout);
        Err(TransportError::Network(
            "token ID endpoint (/tokenize) is unavailable on this transport".to_owned(),
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

    fn post_local_json(
        &self,
        validated_url: &str,
        body: &str,
        timeout: Duration,
    ) -> Result<RawHttpResponse, TransportError> {
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .max_redirects(0)
            .proxy(None)
            .build()
            .into();
        let resp = agent
            .post(validated_url)
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
        self.post_local_json(&validated_endpoint, body, timeout)
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
        let resp = self.post_local_json(&validated_url, &payload, timeout)?;
        if resp.status != 200 {
            return Err(TransportError::Network(format!(
                "POST /tokenize returned HTTP {}",
                resp.status
            )));
        }
        let parsed: serde_json::Value = serde_json::from_str(&resp.body).map_err(|_| {
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

    fn apply_template(
        &self,
        apply_template_url: &str,
        body: &str,
        timeout: Duration,
    ) -> Result<String, TransportError> {
        let (validated_url, _) = validate_local_apply_template_url(
            "local",
            apply_template_url,
            self.gate.allowed_vm_hosts(),
        )
        .map_err(|e| TransportError::Network(e.to_string()))?;
        let resp = self.post_local_json(&validated_url, body, timeout)?;
        if resp.status != 200 {
            return Err(TransportError::Network(format!(
                "POST /apply-template returned HTTP {}",
                resp.status
            )));
        }
        let parsed: serde_json::Value = serde_json::from_str(&resp.body).map_err(|_| {
            TransportError::Network("POST /apply-template returned invalid JSON".to_owned())
        })?;
        let Some(prompt) = parsed.get("prompt").and_then(serde_json::Value::as_str) else {
            return Err(TransportError::Network(
                "POST /apply-template response missing string 'prompt'".to_owned(),
            ));
        };
        Ok(prompt.to_owned())
    }

    fn tokenize_prompt(
        &self,
        tokenize_url: &str,
        content: &str,
        add_special: bool,
        parse_special: bool,
        timeout: Duration,
    ) -> Result<Vec<u64>, TransportError> {
        let (validated_url, _) =
            validate_local_tokenize_url("local", tokenize_url, self.gate.allowed_vm_hosts())
                .map_err(|e| TransportError::Network(e.to_string()))?;
        let payload = serde_json::json!({
            "content": content,
            "add_special": add_special,
            "parse_special": parse_special,
        })
        .to_string();
        let resp = self.post_local_json(&validated_url, &payload, timeout)?;
        if resp.status != 200 {
            return Err(TransportError::Network(format!(
                "POST /tokenize returned HTTP {}",
                resp.status
            )));
        }
        let parsed: serde_json::Value = serde_json::from_str(&resp.body).map_err(|_| {
            TransportError::Network("POST /tokenize returned invalid JSON".to_owned())
        })?;
        let Some(arr) = parsed.get("tokens").and_then(serde_json::Value::as_array) else {
            return Err(TransportError::Network(
                "POST /tokenize response missing array 'tokens'".to_owned(),
            ));
        };
        let ids: Vec<u64> = arr
            .iter()
            .map(serde_json::Value::as_u64)
            .collect::<Option<Vec<u64>>>()
            .ok_or_else(|| {
                TransportError::Network(
                    "POST /tokenize 'tokens' entries must all be non-negative integers".to_owned(),
                )
            })?;
        if ids.is_empty() {
            return Err(TransportError::Network(
                "POST /tokenize returned empty 'tokens' array".to_owned(),
            ));
        }
        Ok(ids)
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
    #[serde(default)]
    apply_template_prompt: Option<String>,
    #[serde(default)]
    tokenize_ids: Option<Vec<u64>>,
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
    apply_template_prompt: Option<String>,
    tokenize_ids: Option<Vec<u64>>,
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
        Ok(Self {
            rules,
            apply_template_prompt: parsed.apply_template_prompt,
            tokenize_ids: parsed.tokenize_ids,
        })
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

    fn apply_template(
        &self,
        _apply_template_url: &str,
        _body: &str,
        _timeout: Duration,
    ) -> Result<String, TransportError> {
        self.apply_template_prompt.clone().ok_or_else(|| {
            TransportError::Network(
                "POST /apply-template is not configured in mock responses.json".to_owned(),
            )
        })
    }

    fn tokenize_prompt(
        &self,
        _tokenize_url: &str,
        _content: &str,
        _add_special: bool,
        _parse_special: bool,
        _timeout: Duration,
    ) -> Result<Vec<u64>, TransportError> {
        self.tokenize_ids
            .clone()
            .filter(|ids| !ids.is_empty())
            .ok_or_else(|| {
                TransportError::Network(
                    "POST /tokenize token IDs are not configured in mock responses.json".to_owned(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    struct MinimalTransport;

    impl Transport for MinimalTransport {
        fn post_chat_completions(
            &self,
            _url: &str,
            _body: &str,
            _timeout: Duration,
        ) -> Result<RawHttpResponse, TransportError> {
            Err(TransportError::Network("unused".to_owned()))
        }
    }

    fn spawn_scripted_http(responses: Vec<(u16, String)>) -> (String, thread::JoinHandle<()>) {
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(e) => panic!("bind 127.0.0.1:0: {e}"),
        };
        let addr = match listener.local_addr() {
            Ok(a) => a,
            Err(e) => panic!("local_addr: {e}"),
        };
        let handle = thread::spawn(move || {
            let mut queue = VecDeque::from(responses);
            while let Some((status, body)) = queue.pop_front() {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) => panic!("accept: {e}"),
                };
                let mut buf = Vec::new();
                let mut tmp = [0_u8; 512];
                let mut header_end = None;
                while header_end.is_none() {
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            header_end =
                                buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
                        }
                        Err(_) => break,
                    }
                }
                if let Some(hdr_end) = header_end {
                    let hdr_str = String::from_utf8_lossy(&buf[..hdr_end]);
                    let content_len = hdr_str
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            if k.eq_ignore_ascii_case("Content-Length") {
                                v.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    let mut remaining = (hdr_end + content_len).saturating_sub(buf.len());
                    while remaining > 0 {
                        let chunk = remaining.min(tmp.len());
                        match stream.read(&mut tmp[..chunk]) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => remaining -= n,
                        }
                    }
                }
                let wire = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(wire.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("http://127.0.0.1:{}", addr.port()), handle)
    }

    #[test]
    fn local_and_mock_transport_apply_template_and_tokenize_cover_ok_and_error_paths() {
        let minimal = MinimalTransport;
        let timeout = Duration::from_secs(2);
        assert!(
            minimal
                .apply_template("http://127.0.0.1:1/apply-template", "{}", timeout)
                .is_err()
        );
        assert!(
            minimal
                .tokenize_prompt("http://127.0.0.1:1/tokenize", "hi", true, true, timeout)
                .is_err()
        );

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "ml_transport_mock_test_{}_{stamp}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        if let Err(e) = fs::create_dir_all(&dir) {
            panic!("mkdir {}: {e}", dir.display());
        }
        assert!(MockDirTransport::load(&dir).is_err());

        let resp_path = dir.join("responses.json");
        if let Err(e) = fs::write(&resp_path, r#"{"rules":[]}"#) {
            panic!("write empty mock: {e}");
        }
        let empty_mock = match MockDirTransport::load(&dir) {
            Ok(m) => m,
            Err(e) => panic!("load empty mock: {e}"),
        };
        assert!(
            empty_mock
                .apply_template("http://127.0.0.1:1/apply-template", "{}", timeout)
                .is_err()
        );
        assert!(
            empty_mock
                .tokenize_prompt("http://127.0.0.1:1/tokenize", "x", true, true, timeout)
                .is_err()
        );

        if let Err(e) = fs::write(
            &resp_path,
            r#"{"rules":[],"apply_template_prompt":"TPL_OK","tokenize_ids":[5,6,7]}"#,
        ) {
            panic!("write populated mock: {e}");
        }
        let pop_mock = match MockDirTransport::load(&dir) {
            Ok(m) => m,
            Err(e) => panic!("load populated mock: {e}"),
        };
        assert_eq!(
            pop_mock
                .apply_template("http://127.0.0.1:1/apply-template", "{}", timeout)
                .ok()
                .as_deref(),
            Some("TPL_OK")
        );
        assert_eq!(
            pop_mock
                .tokenize_prompt("http://127.0.0.1:1/tokenize", "x", true, true, timeout)
                .ok(),
            Some(vec![5, 6, 7])
        );
        let _ = fs::remove_dir_all(&dir);

        let gate = match LocalGate::acquire(false, None, false, &[]) {
            Ok(g) => g,
            Err(e) => panic!("LocalGate: {e}"),
        };
        let tx = LocalOpenAiTransport::new(gate);

        assert!(
            tx.apply_template("https://example.com/apply-template", "{}", timeout)
                .is_err()
        );
        assert!(
            tx.tokenize_prompt("https://example.com/tokenize", "p", true, true, timeout)
                .is_err()
        );

        let (base, handle) = spawn_scripted_http(vec![
            (500, "err".to_owned()),
            (200, "not-json".to_owned()),
            (200, r#"{"other":"x"}"#.to_owned()),
            (200, r#"{"prompt":"RENDERED"}"#.to_owned()),
            (500, "err".to_owned()),
            (200, "not-json".to_owned()),
            (200, r#"{"other":1}"#.to_owned()),
            (200, r#"{"tokens":["bad"]}"#.to_owned()),
            (200, r#"{"tokens":[]}"#.to_owned()),
            (200, r#"{"tokens":[10,20,30]}"#.to_owned()),
        ]);

        let tpl_url = format!("{base}/apply-template");
        let tok_url = format!("{base}/tokenize");
        assert!(tx.apply_template(&tpl_url, "{}", timeout).is_err());
        assert!(tx.apply_template(&tpl_url, "{}", timeout).is_err());
        assert!(tx.apply_template(&tpl_url, "{}", timeout).is_err());
        assert_eq!(
            tx.apply_template(&tpl_url, "{}", timeout).ok().as_deref(),
            Some("RENDERED")
        );

        assert!(
            tx.tokenize_prompt(&tok_url, "RENDERED", true, true, timeout)
                .is_err()
        );
        assert!(
            tx.tokenize_prompt(&tok_url, "RENDERED", true, true, timeout)
                .is_err()
        );
        assert!(
            tx.tokenize_prompt(&tok_url, "RENDERED", true, true, timeout)
                .is_err()
        );
        assert!(
            tx.tokenize_prompt(&tok_url, "RENDERED", true, true, timeout)
                .is_err()
        );
        assert!(
            tx.tokenize_prompt(&tok_url, "RENDERED", true, true, timeout)
                .is_err()
        );
        assert_eq!(
            tx.tokenize_prompt(&tok_url, "RENDERED", true, true, timeout)
                .ok(),
            Some(vec![10, 20, 30])
        );
        let _ = handle.join();
        assert!(tx.apply_template(&tpl_url, "{}", timeout).is_err());
    }
}

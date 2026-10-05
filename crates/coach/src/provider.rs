//! AI provider interface and the OpenAI-compatible chat-completions adapter.
//!
//! The default configuration targets a *local* model server that is free to
//! run — Ollama (`http://localhost:11434/v1`) or LM Studio
//! (`http://localhost:1234/v1`) — so no data leaves the computer and nothing
//! is billed. A remote endpoint with a user-supplied key is supported for the
//! owner's later use but ships disabled, and it was not live-tested (spec
//! no-payment restriction).

use rl_json::{parse, Value};
use rl_net::http::{is_local_host, parse_url, HttpClient, HttpError, HttpRequest};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(c: impl Into<String>) -> Self {
        ChatMessage { role: "system".into(), content: c.into() }
    }
    pub fn user(c: impl Into<String>) -> Self {
        ChatMessage { role: "user".into(), content: c.into() }
    }
    pub fn assistant(c: impl Into<String>) -> Self {
        ChatMessage { role: "assistant".into(), content: c.into() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiReply {
    pub text: String,
    pub model: String,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AiError {
    /// Service not reachable (e.g. Ollama not running).
    Unavailable(String),
    Timeout,
    Http(u16, String),
    BadResponse(String),
    /// Blocked locally: consent off, daily limit reached, remote disabled.
    Refused(String),
    Cancelled,
}

impl AiError {
    pub fn user_message(&self) -> String {
        match self {
            AiError::Unavailable(m) => format!("The AI service isn't reachable ({m}). If you use Ollama, make sure it is running."),
            AiError::Timeout => "The AI service took too long to answer. Local models can be slow on the first request; try again or use a smaller model.".into(),
            AiError::Http(401, _) | AiError::Http(403, _) => "The AI service rejected the request (check the API key).".into(),
            AiError::Http(404, _) => "The AI service doesn't know that model or endpoint. Check the model name in Settings (for Ollama: run `ollama pull <model>`).".into(),
            AiError::Http(c, m) => format!("The AI service returned an error (HTTP {c}): {}", m.chars().take(160).collect::<String>()),
            AiError::BadResponse(m) => format!("The AI service sent an unexpected response: {m}"),
            AiError::Refused(m) => m.clone(),
            AiError::Cancelled => "Cancelled.".into(),
        }
    }
}

pub trait AiProvider: Send + Sync {
    fn id(&self) -> String;
    fn model(&self) -> String;
    fn is_local(&self) -> bool;
    fn complete(&self, http: &dyn HttpClient, msgs: &[ChatMessage], json: bool, max_tokens: u32, timeout: Duration, cancel: &AtomicBool) -> Result<AiReply, AiError>;
    fn list_models(&self, http: &dyn HttpClient) -> Result<Vec<String>, AiError>;
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatible {
    pub label: String,
    /// Base URL including the API version path, e.g. http://localhost:11434/v1
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
}

impl OpenAiCompatible {
    pub fn ollama(model: &str) -> Self {
        OpenAiCompatible { label: "Ollama (local)".into(), base_url: "http://localhost:11434/v1".into(), model: model.into(), api_key: None }
    }

    pub fn request_body(&self, msgs: &[ChatMessage], json: bool, max_tokens: u32) -> String {
        let mut v = Value::obj([
            ("model", self.model.clone().into()),
            ("messages", Value::Arr(msgs.iter().map(|m| Value::obj([("role", m.role.clone().into()), ("content", m.content.clone().into())])).collect())),
            ("temperature", 0.2.into()),
            ("max_tokens", max_tokens.into()),
            ("stream", false.into()),
        ]);
        if json {
            v.set("response_format", Value::obj([("type", "json_object".into())]));
        }
        v.to_string_compact()
    }

    pub fn parse_response(body: &str) -> Result<(String, String, Option<u64>, Option<u64>), AiError> {
        let v = parse(body).map_err(|e| AiError::BadResponse(format!("not JSON ({e})")))?;
        if let Some(e) = v.get("error") {
            let m = e.get("message").and_then(|x| x.as_str()).or_else(|| e.as_str()).unwrap_or("unknown error");
            return Err(AiError::BadResponse(m.chars().take(200).collect()));
        }
        let content = v
            .get("choices")
            .and_then(|c| c.as_arr())
            .and_then(|c| c.first())
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| AiError::BadResponse("no message content".into()))?;
        let model = v.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();
        let usage = v.get("usage");
        let pt = usage.and_then(|u| u.get("prompt_tokens")).and_then(|x| x.as_i64()).map(|x| x.max(0) as u64);
        let ct = usage.and_then(|u| u.get("completion_tokens")).and_then(|x| x.as_i64()).map(|x| x.max(0) as u64);
        Ok((content.to_string(), model, pt, ct))
    }
}

fn map_http(e: HttpError) -> AiError {
    match e {
        HttpError::Timeout => AiError::Timeout,
        HttpError::Cancelled => AiError::Cancelled,
        HttpError::Connect(m) => AiError::Unavailable(m),
        HttpError::Unsupported(m) => AiError::Refused(m),
        HttpError::Protocol(m) => AiError::BadResponse(m),
    }
}

impl AiProvider for OpenAiCompatible {
    fn id(&self) -> String {
        self.label.clone()
    }
    fn model(&self) -> String {
        self.model.clone()
    }
    fn is_local(&self) -> bool {
        parse_url(&self.base_url).map(|u| is_local_host(&u.host)).unwrap_or(false)
    }
    fn complete(&self, http: &dyn HttpClient, msgs: &[ChatMessage], json: bool, max_tokens: u32, timeout: Duration, cancel: &AtomicBool) -> Result<AiReply, AiError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(AiError::Cancelled);
        }
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let mut req = HttpRequest::post_json(&url, self.request_body(msgs, json, max_tokens), timeout);
        if let Some(k) = &self.api_key {
            req = req.header("Authorization", &format!("Bearer {k}"));
        }
        let t0 = Instant::now();
        let resp = http.send(&req).map_err(map_http)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(AiError::Cancelled);
        }
        if resp.status != 200 {
            let msg = parse(&resp.text())
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.get("message").and_then(|m| m.as_str()).or_else(|| e.as_str()).map(|s| s.to_string())))
                .unwrap_or_default();
            return Err(AiError::Http(resp.status, msg));
        }
        let (text, model, pt, ct) = Self::parse_response(&resp.text())?;
        Ok(AiReply { text, model: if model.is_empty() { self.model.clone() } else { model }, prompt_tokens: pt, completion_tokens: ct, latency_ms: t0.elapsed().as_millis() as u64 })
    }
    fn list_models(&self, http: &dyn HttpClient) -> Result<Vec<String>, AiError> {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let mut req = HttpRequest::get(&url, Duration::from_secs(5));
        if let Some(k) = &self.api_key {
            req = req.header("Authorization", &format!("Bearer {k}"));
        }
        let resp = http.send(&req).map_err(map_http)?;
        if resp.status != 200 {
            return Err(AiError::Http(resp.status, String::new()));
        }
        let v = parse(&resp.text()).map_err(|e| AiError::BadResponse(e.0))?;
        Ok(v.get("data").and_then(|d| d.as_arr()).map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(|s| s.to_string())).collect()).unwrap_or_default())
    }
}

/// Extract the first top-level JSON object from model output that may be
/// wrapped in prose or Markdown code fences.
pub fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let b = text.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for i in start..b.len() {
        let c = b[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_net::http::MockHttp;

    #[test]
    fn request_and_response_shapes() {
        let p = OpenAiCompatible::ollama("llama3.2:3b");
        assert!(p.is_local());
        let body = p.request_body(&[ChatMessage::system("s"), ChatMessage::user("u")], true, 500);
        assert!(body.contains("\"response_format\":{\"type\":\"json_object\"}"));
        let resp = r#"{"id":"x","model":"llama3.2:3b","choices":[{"index":0,"message":{"role":"assistant","content":"{\"a\":1}"}}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
        let mock = MockHttp::new(vec![MockHttp::ok(resp)]);
        let r = p.complete(&mock, &[ChatMessage::user("hi")], true, 100, Duration::from_secs(5), &AtomicBool::new(false)).unwrap();
        assert_eq!(r.text, "{\"a\":1}");
        assert_eq!(r.prompt_tokens, Some(10));
        assert!(mock.requests.lock().unwrap()[0].url.ends_with("/v1/chat/completions"));
        let remote = OpenAiCompatible { label: "x".into(), base_url: "https://api.example.com/v1".into(), model: "m".into(), api_key: Some("k".into()) };
        assert!(!remote.is_local());
    }

    #[test]
    fn errors_map_to_helpful_messages() {
        let p = OpenAiCompatible::ollama("missing");
        let mock = MockHttp::new(vec![Ok(rl_net::http::HttpResponse { status: 404, body: br#"{"error":{"message":"model not found"}}"#.to_vec() })]);
        let e = p.complete(&mock, &[ChatMessage::user("hi")], false, 10, Duration::from_secs(1), &AtomicBool::new(false)).unwrap_err();
        assert!(e.user_message().contains("ollama pull"));
        let mock = MockHttp::new(vec![Err(HttpError::Connect("refused".into()))]);
        let e = p.complete(&mock, &[ChatMessage::user("hi")], false, 10, Duration::from_secs(1), &AtomicBool::new(false)).unwrap_err();
        assert!(matches!(e, AiError::Unavailable(_)));
        assert_eq!(p.complete(&mock, &[], false, 1, Duration::from_secs(1), &AtomicBool::new(true)).unwrap_err(), AiError::Cancelled);
    }

    #[test]
    fn json_extraction() {
        assert_eq!(extract_json_object("Sure!\n```json\n{\"a\":{\"b\":\"}\"}}\n```"), Some("{\"a\":{\"b\":\"}\"}}"));
        assert_eq!(extract_json_object("no json"), None);
    }
}

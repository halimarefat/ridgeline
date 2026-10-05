//! TLS-capable HTTP client for the desktop build (ureq + rustls).
//! Used for OpenFreeMap-independent services: routing, elevation and the
//! optional AI endpoint. Never used in the trainer control path.

use rl_net::http::{HttpClient, HttpError, HttpRequest, HttpResponse, MAX_BODY_BYTES};
use std::io::Read;
use std::time::Duration;

pub struct UreqClient {
    agent: ureq::Agent,
}

impl UreqClient {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .user_agent(concat!("Ridgeline/", env!("CARGO_PKG_VERSION"), " (+https://github.com/halimarefat/ridgeline)"))
            .redirects(3)
            .build();
        UreqClient { agent }
    }
}

fn read_body(resp: ureq::Response) -> Result<Vec<u8>, HttpError> {
    let mut body = Vec::new();
    resp.into_reader()
        .take(MAX_BODY_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|e| if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock { HttpError::Timeout } else { HttpError::Protocol(e.to_string()) })?;
    if body.len() > MAX_BODY_BYTES {
        return Err(HttpError::Protocol("response too large".into()));
    }
    Ok(body)
}

impl HttpClient for UreqClient {
    fn supports_tls(&self) -> bool {
        true
    }
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let mut r = self.agent.request(&req.method, &req.url).timeout(req.timeout);
        for (k, v) in &req.headers {
            r = r.set(k, v);
        }
        let res = match &req.body {
            Some(b) => r.send_bytes(b),
            None => r.call(),
        };
        match res {
            Ok(resp) => {
                let status = resp.status();
                Ok(HttpResponse { status, body: read_body(resp)? })
            }
            Err(ureq::Error::Status(status, resp)) => Ok(HttpResponse { status, body: read_body(resp).unwrap_or_default() }),
            Err(ureq::Error::Transport(t)) => {
                let msg = t.to_string();
                Err(match t.kind() {
                    ureq::ErrorKind::Dns | ureq::ErrorKind::ConnectionFailed => HttpError::Connect(msg),
                    ureq::ErrorKind::Io if msg.contains("timed out") => HttpError::Timeout,
                    ureq::ErrorKind::Io => HttpError::Connect(msg),
                    _ => HttpError::Protocol(msg),
                })
            }
        }
    }
}

//! Minimal HTTP client abstraction.
//!
//! The core never opens sockets directly: providers receive an
//! `HttpClient`. The desktop build supplies a TLS-capable client (ureq +
//! rustls); this crate ships a dependency-free plain-HTTP client that is
//! sufficient for local services such as Ollama or LM Studio on
//! `http://localhost` and for tests.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
}

impl HttpRequest {
    pub fn get(url: &str, timeout: Duration) -> Self {
        HttpRequest { method: "GET".into(), url: url.into(), headers: vec![], body: None, timeout }
    }
    pub fn post_json(url: &str, body: String, timeout: Duration) -> Self {
        HttpRequest { method: "POST".into(), url: url.into(), headers: vec![("Content-Type".into(), "application/json".into())], body: Some(body.into_bytes()), timeout }
    }
    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HttpError {
    /// Could not connect (service not running, offline, DNS).
    Connect(String),
    Timeout,
    /// TLS URL given to a client without TLS support.
    Unsupported(String),
    Protocol(String),
    Cancelled,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Connect(m) => write!(f, "could not connect: {m}"),
            HttpError::Timeout => write!(f, "request timed out"),
            HttpError::Unsupported(m) => write!(f, "{m}"),
            HttpError::Protocol(m) => write!(f, "bad response: {m}"),
            HttpError::Cancelled => write!(f, "cancelled"),
        }
    }
}

pub trait HttpClient: Send + Sync {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError>;
    /// Whether https:// URLs are supported.
    fn supports_tls(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn parse_url(u: &str) -> Result<Url, String> {
    let (scheme, rest) = u.split_once("://").ok_or("URL must start with http:// or https://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err("Only http and https URLs are supported.".into());
    }
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if hostport.contains('@') {
        return Err("Credentials in URLs are not allowed.".into());
    }
    let (host, port) = if hostport.starts_with('[') {
        let end = hostport.find(']').ok_or("bad IPv6 host")?;
        let h = &hostport[1..end];
        let p = hostport[end + 1..].strip_prefix(':').map(|p| p.parse::<u16>().map_err(|_| "bad port")).transpose()?;
        (h.to_string(), p)
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p.parse::<u16>().map_err(|_| "bad port")?)),
            None => (hostport.to_string(), None),
        }
    };
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("bad host".into());
    }
    let port = port.unwrap_or(if scheme == "https" { 443 } else { 80 });
    Ok(Url { scheme, host, port, path: path.to_string() })
}

/// True for loopback hosts (local AI servers).
pub fn is_local_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "localhost" || h == "127.0.0.1" || h == "::1" || h.starts_with("127.")
}

pub struct StdHttpClient;

impl HttpClient for StdHttpClient {
    fn supports_tls(&self) -> bool {
        false
    }
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let url = parse_url(&req.url).map_err(HttpError::Protocol)?;
        if url.scheme != "http" {
            return Err(HttpError::Unsupported("This build cannot make HTTPS requests from the core; use the desktop app.".into()));
        }
        let addr = (url.host.as_str(), url.port)
            .to_socket_addrs()
            .map_err(|e| HttpError::Connect(e.to_string()))?
            .next()
            .ok_or_else(|| HttpError::Connect("no address".into()))?;
        let mut s = TcpStream::connect_timeout(&addr, req.timeout.min(Duration::from_secs(10))).map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                HttpError::Timeout
            } else {
                HttpError::Connect(e.to_string())
            }
        })?;
        s.set_read_timeout(Some(req.timeout)).ok();
        s.set_write_timeout(Some(req.timeout)).ok();
        let host_hdr = if url.port == 80 { url.host.clone() } else { format!("{}:{}", url.host, url.port) };
        let mut head = format!("{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept-Encoding: identity\r\nUser-Agent: Ridgeline/0.1\r\n", req.method, url.path, host_hdr);
        for (k, v) in &req.headers {
            if k.contains(['\r', '\n']) || v.contains(['\r', '\n']) {
                return Err(HttpError::Protocol("invalid header".into()));
            }
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        if let Some(b) = &req.body {
            head.push_str(&format!("Content-Length: {}\r\n", b.len()));
        }
        head.push_str("\r\n");
        let map_io = |e: std::io::Error| if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) { HttpError::Timeout } else { HttpError::Connect(e.to_string()) };
        s.write_all(head.as_bytes()).map_err(map_io)?;
        if let Some(b) = &req.body {
            s.write_all(b).map_err(map_io)?;
        }
        let mut r = BufReader::new(s);
        let mut status_line = String::new();
        r.read_line(&mut status_line).map_err(map_io)?;
        let status: u16 = status_line.split_whitespace().nth(1).and_then(|x| x.parse().ok()).ok_or_else(|| HttpError::Protocol("bad status line".into()))?;
        let mut content_length: Option<usize> = None;
        let mut chunked = false;
        loop {
            let mut line = String::new();
            let n = r.read_line(&mut line).map_err(map_io)?;
            if n == 0 || line == "\r\n" || line == "\n" {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                let k = k.trim().to_ascii_lowercase();
                let v = v.trim();
                if k == "content-length" {
                    content_length = v.parse().ok();
                } else if k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked") {
                    chunked = true;
                }
            }
        }
        let mut body = Vec::new();
        if chunked {
            loop {
                let mut sz = String::new();
                r.read_line(&mut sz).map_err(map_io)?;
                let n = usize::from_str_radix(sz.trim().split(';').next().unwrap_or(""), 16).map_err(|_| HttpError::Protocol("bad chunk".into()))?;
                if n == 0 {
                    break;
                }
                if body.len() + n > MAX_BODY_BYTES {
                    return Err(HttpError::Protocol("response too large".into()));
                }
                let mut chunk = vec![0u8; n];
                r.read_exact(&mut chunk).map_err(map_io)?;
                body.extend_from_slice(&chunk);
                let mut crlf = [0u8; 2];
                r.read_exact(&mut crlf).map_err(map_io)?;
            }
        } else if let Some(n) = content_length {
            if n > MAX_BODY_BYTES {
                return Err(HttpError::Protocol("response too large".into()));
            }
            body.resize(n, 0);
            r.read_exact(&mut body).map_err(map_io)?;
        } else {
            r.take(MAX_BODY_BYTES as u64).read_to_end(&mut body).map_err(map_io)?;
        }
        Ok(HttpResponse { status, body })
    }
}

/// Scripted client for tests: returns queued responses in order.
pub struct MockHttp {
    pub responses: std::sync::Mutex<Vec<Result<HttpResponse, HttpError>>>,
    pub requests: std::sync::Mutex<Vec<HttpRequest>>,
}

impl MockHttp {
    pub fn new(responses: Vec<Result<HttpResponse, HttpError>>) -> Self {
        MockHttp { responses: std::sync::Mutex::new(responses), requests: std::sync::Mutex::new(vec![]) }
    }
    pub fn ok(body: &str) -> Result<HttpResponse, HttpError> {
        Ok(HttpResponse { status: 200, body: body.as_bytes().to_vec() })
    }
}

impl HttpClient for MockHttp {
    fn supports_tls(&self) -> bool {
        true
    }
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        self.requests.lock().unwrap().push(req.clone());
        let mut r = self.responses.lock().unwrap();
        if r.is_empty() {
            Err(HttpError::Connect("no scripted response".into()))
        } else {
            r.remove(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn url_parsing() {
        let u = parse_url("http://localhost:11434/v1/chat/completions").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("localhost", 11434, "/v1/chat/completions"));
        assert_eq!(parse_url("https://example.org").unwrap().port, 443);
        assert!(parse_url("ftp://x").is_err());
        assert!(parse_url("http://user:pw@host/").is_err());
        assert!(is_local_host("127.0.0.1") && !is_local_host("example.org"));
    }

    #[test]
    fn std_client_handles_chunked_and_length() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            for (i, conn) in l.incoming().take(2).enumerate() {
                let mut c = conn.unwrap();
                let mut buf = [0u8; 4096];
                let _ = c.read(&mut buf);
                let resp = if i == 0 {
                    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n".to_string()
                } else {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 3\r\n\r\nnop".to_string()
                };
                c.write_all(resp.as_bytes()).unwrap();
            }
        });
        let c = StdHttpClient;
        let r = c.send(&HttpRequest::post_json(&format!("http://127.0.0.1:{port}/x"), "{}".into(), Duration::from_secs(5))).unwrap();
        assert_eq!((r.status, r.text().as_str()), (200, "hello world"));
        let r = c.send(&HttpRequest::get(&format!("http://127.0.0.1:{port}/y"), Duration::from_secs(5))).unwrap();
        assert_eq!((r.status, r.text().as_str()), (404, "nop"));
        h.join().unwrap();
        // Nothing listening: connect error, not a hang.
        let e = c.send(&HttpRequest::get("http://127.0.0.1:1/", Duration::from_secs(2))).unwrap_err();
        assert!(matches!(e, HttpError::Connect(_)), "{e:?}");
        assert!(matches!(c.send(&HttpRequest::get("https://example.org/", Duration::from_secs(2))), Err(HttpError::Unsupported(_))));
    }
}

//! Ridgeline development server.
//!
//! Serves the built UI and the same JSON command interface the desktop app
//! uses, so the interface can be developed and tested in a normal browser
//! with the simulator. It is a developer tool: it binds to 127.0.0.1 only,
//! has no Bluetooth access (simulated devices only) and only plain-HTTP
//! networking (enough for a local Ollama). Riders use the desktop app.
//!
//! Usage: rl-devserver [--port 1420] [--data DIR] [--ui DIR] [--exports DIR]

use rl_app::{Config, NullPlatform, Runtime};
use rl_json::{parse, Value};
use rl_net::http::StdHttpClient;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn content_type(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "map" => "application/json",
        _ => "application/octet-stream",
    }
}

fn respond(s: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

fn handle(mut s: TcpStream, rt: Arc<Runtime>, ui: PathBuf, port: u16) {
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(30)));
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    if r.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("/").to_string());
    let mut len = 0usize;
    let mut host_ok = false;
    let mut origin_ok = true;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            match k.as_str() {
                "content-length" => len = v.parse().unwrap_or(0),
                // DNS-rebinding protection: only loopback host names.
                "host" => host_ok = v == format!("127.0.0.1:{port}") || v == format!("localhost:{port}"),
                "origin" => origin_ok = v == format!("http://127.0.0.1:{port}") || v == format!("http://localhost:{port}"),
                _ => {}
            }
        }
    }
    if !host_ok || !origin_ok {
        respond(&mut s, "403 Forbidden", "text/plain", b"forbidden");
        return;
    }
    if method == "POST" && target == "/rpc" {
        if len > 30 * 1024 * 1024 {
            respond(&mut s, "413 Payload Too Large", "text/plain", b"too large");
            return;
        }
        let mut body = vec![0u8; len];
        if r.read_exact(&mut body).is_err() {
            return;
        }
        let text = String::from_utf8_lossy(&body);
        let reply = match parse(&text) {
            Ok(v) => {
                let m = v.get("method").and_then(|m| m.as_str()).unwrap_or("").to_string();
                let p = v.get("params").cloned().unwrap_or_else(Value::empty_obj);
                rt.rpc(&m, &p.to_string_compact())
            }
            Err(e) => Value::obj([("ok", false.into()), ("error", format!("bad request: {e}").into())]).to_string_compact(),
        };
        respond(&mut s, "200 OK", "application/json", reply.as_bytes());
        return;
    }
    if method != "GET" {
        respond(&mut s, "405 Method Not Allowed", "text/plain", b"method not allowed");
        return;
    }
    let path = target.split('?').next().unwrap_or("/");
    let rel = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
    if rel.split('/').any(|c| c == ".." || c.is_empty()) {
        respond(&mut s, "400 Bad Request", "text/plain", b"bad path");
        return;
    }
    let file = ui.join(rel);
    match std::fs::read(&file) {
        Ok(b) => respond(&mut s, "200 OK", content_type(&file), &b),
        Err(_) => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
    }
}

fn main() {
    let port: u16 = arg("--port").and_then(|p| p.parse().ok()).unwrap_or(1420);
    let data = PathBuf::from(arg("--data").unwrap_or_else(|| ".ridgeline-data".into()));
    let exports = PathBuf::from(arg("--exports").unwrap_or_else(|| data.join("exports").display().to_string()));
    let ui = PathBuf::from(arg("--ui").unwrap_or_else(|| "apps/desktop/ui/dist".into()));
    let cfg = Config { data_dir: data.clone(), export_dir: exports, platform_name: format!("devserver-{}", std::env::consts::OS), app_version: format!("{}-dev", rl_app::APP_VERSION) };
    let rt = match Runtime::start(cfg, vec![], Arc::new(StdHttpClient), Arc::new(NullPlatform)) {
        Ok(r) => Arc::new(r),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|e| {
        eprintln!("error: cannot listen on 127.0.0.1:{port}: {e}");
        std::process::exit(1);
    });
    println!("Ridgeline dev server: http://127.0.0.1:{port}  (data: {}, ui: {})", data.display(), ui.display());
    println!("Simulated devices only. Press Ctrl+C to stop.");
    for conn in listener.incoming().flatten() {
        let (rt, ui) = (rt.clone(), ui.clone());
        std::thread::spawn(move || handle(conn, rt, ui, port));
    }
}

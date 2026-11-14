//! `openpdn-devserver [--port N] [--root DIR]`
//!
//! Serves the web UI and, on `/ws`, runs an [`openpdn_editor::Editor`]
//! natively for that connection. The UI talks to it exactly as it talks to
//! the wasm engine in a worker, which lets the whole app be exercised
//! without a WebAssembly build. Also sends the COOP/COEP headers that the
//! threaded wasm build needs.
//!
//! Frames (both directions, binary): u32 LE JSON length, JSON, payload.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Instant;

use openpdn_editor::Session;
use openpdn_formats::json::Json;
use tungstenite::Message;

fn content_type(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "wasm" => "application/wasm",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn serve_file(mut s: TcpStream, root: &Path, path: &str) {
    let rel = path.split(['?', '#']).next().unwrap_or("/").trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let mut full = PathBuf::from(root);
    for part in rel.split('/') {
        if part == ".." || part.is_empty() {
            let _ = s.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
            return;
        }
        full.push(part);
    }
    let (status, body, ct) = match std::fs::read(&full) {
        Ok(b) => ("200 OK", b, content_type(&full)),
        Err(_) => ("404 Not Found", b"not found".to_vec(), "text/plain"),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         Cross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Embedder-Policy: require-corp\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes()).and_then(|_| s.write_all(&body));
}

fn frame(json: &Json, bin: &[u8]) -> Vec<u8> {
    let j = json.to_string_pretty();
    let mut out = Vec::with_capacity(4 + j.len() + bin.len());
    out.extend_from_slice(&(j.len() as u32).to_le_bytes());
    out.extend_from_slice(j.as_bytes());
    out.extend_from_slice(bin);
    out
}

fn run_socket(s: TcpStream, verbose: bool) {
    let Ok(mut ws) = tungstenite::accept(s) else { return };
    let mut editor = Session::default();
    loop {
        let msg = match ws.read() {
            Ok(Message::Binary(b)) => b,
            Ok(Message::Text(t)) => {
                let mut v = (t.len() as u32).to_le_bytes().to_vec();
                v.extend_from_slice(t.as_bytes());
                v
            }
            Ok(Message::Close(_)) | Err(_) => return,
            Ok(_) => continue,
        };
        if msg.len() < 4 {
            continue;
        }
        let n = u32::from_le_bytes(msg[..4].try_into().unwrap()) as usize;
        let Some(text) = msg.get(4..4 + n).and_then(|b| std::str::from_utf8(b).ok()) else { continue };
        let Ok(req) = Json::parse(text) else { continue };
        let started = Instant::now();
        let reply = editor.handle(&req, &msg[4 + n..]);
        if verbose {
            let cmd = req.get("cmd").and_then(Json::as_str).unwrap_or("?");
            eprintln!("{:>8.2} ms  {cmd}", started.elapsed().as_secs_f64() * 1000.0);
        }
        if ws.send(Message::Binary(frame(&reply.to_json(), &reply.binary()))).is_err() {
            return;
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let port: u16 = get("--port").and_then(|p| p.parse().ok()).unwrap_or(8090);
    let root = PathBuf::from(get("--root").unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../web").into()));
    let verbose = args.iter().any(|a| a == "-v");
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    eprintln!("PaintDotRust dev server: http://127.0.0.1:{port}/  (root {})", root.display());
    for stream in listener.incoming().flatten() {
        let root = root.clone();
        std::thread::spawn(move || {
            // Peek the request line to route without consuming the handshake.
            let mut buf = [0u8; 2048];
            let n = stream.peek(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]);
            let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
            if verbose {
                eprintln!("request {path} ({n} bytes peeked)");
            }
            if path.starts_with("/ws") {
                run_socket(stream, verbose);
            } else {
                // Drain the request head before answering.
                let mut r = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                while r.read_line(&mut line).map(|n| n > 2).unwrap_or(false) {
                    line.clear();
                }
                if verbose {
                    eprintln!("head read");
                }
                serve_file(stream, &root, &path);
                if verbose {
                    eprintln!("served");
                }
            }
        });
    }
}

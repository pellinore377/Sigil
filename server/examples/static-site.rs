//! A static website for tests, std only: `static-site <bind:port> <dir>`
//! answers GET with the file under <dir> (a directory: its index.html),
//! 404 otherwise, and logs each request as `GET /path` on stdout.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;

fn content_type(p: &Path, body: &[u8]) -> &'static str {
    match p.extension().and_then(|e| e.to_str()) {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("json") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        Some("css") => "text/css",
        Some("js") => "text/javascript",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        // a bare pointer like /.well-known/sigil
        None if body.trim_ascii_start().first().is_some_and(|b| matches!(b, b'{' | b'[')) => "application/json",
        _ => "application/octet-stream",
    }
}

fn file(dir: &Path, path: &str) -> Option<(Vec<u8>, &'static str)> {
    if path.split('/').any(|p| p == "..") {
        return None;
    }
    let mut p = dir.join(path.trim_start_matches('/'));
    if p.is_dir() {
        p = p.join("index.html");
    }
    let body = std::fs::read(&p).ok()?;
    let ct = content_type(&p, &body);
    Some((body, ct))
}

fn serve(mut s: TcpStream, dir: &Path) -> std::io::Result<()> {
    let mut rd = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    rd.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").split('?').next().unwrap_or("/").to_string();
    loop {
        let mut h = String::new();
        if rd.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
    }
    println!("{method} {path}");
    let (status, ct, body) = if method != "GET" {
        ("405 Method Not Allowed", "text/plain", b"method not allowed".to_vec())
    } else {
        match file(dir, &path) {
            Some((body, ct)) => ("200 OK", ct, body),
            None => ("404 Not Found", "text/plain", b"not found".to_vec()),
        }
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes())?;
    s.write_all(&body)?;
    s.flush()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (bind, dir) = match args.as_slice() {
        [_, bind, dir] => (bind.clone(), std::path::PathBuf::from(dir)),
        _ => {
            eprintln!("usage: static-site <bind:port> <dir>");
            std::process::exit(2);
        }
    };
    let l = TcpListener::bind(&bind).expect("bind");
    println!("static site at http://{bind} from {}", dir.display());
    for s in l.incoming().flatten() {
        let dir = dir.clone();
        std::thread::spawn(move || {
            let _ = serve(s, &dir);
        });
    }
}

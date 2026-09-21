//! Loopback-only static server for the isolated synthetic video acceptance page.
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener},
    path::PathBuf,
    time::Duration,
};
fn main() -> std::io::Result<()> {
    let root = PathBuf::from(std::env::args().nth(1).expect("fixture directory"));
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    println!("Silent video fixture: http://{}/", listener.local_addr()?);
    for stream in listener.incoming() {
        let mut stream = stream?;
        let root = root.clone();
        std::thread::spawn(move || -> std::io::Result<()> {
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut request = [0u8; 4096];
            let n = stream.read(&mut request)?;
            let text = String::from_utf8_lossy(&request[..n]);
            let path = text
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("GET "))
                .and_then(|line| line.split_whitespace().next())
                .unwrap_or("")
                .split('?')
                .next()
                .unwrap_or("");
            let (name, mime) = match path {
                "/" => ("index.html", "text/html"),
                "/web/sigil_browser.js" => ("web/sigil_browser.js", "text/javascript"),
                "/web/sigil_browser_bg.wasm" => ("web/sigil_browser_bg.wasm", "application/wasm"),
                "/web/sigil-media-worker.mjs" => ("web/sigil-media-worker.mjs", "text/javascript"),
                "/web/sigil-video-control.mjs" => {
                    ("web/sigil-video-control.mjs", "text/javascript")
                }
                _ => {
                    stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )?;
                    return Ok(());
                }
            };
            let bytes = std::fs::read(root.join(name))?;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Embedder-Policy: require-corp\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                bytes.len()
            )?;
            stream.write_all(&bytes)
        });
    }
    Ok(())
}

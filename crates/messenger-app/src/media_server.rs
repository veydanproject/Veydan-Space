// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Files of the messenger for the webview to play and show: a small HTTP
//! server on the loopback, so that a video plays and seeks from the file
//! however large it is. The protocols of the webview do not do that
//! everywhere: Android's player refuses what a custom protocol gives it,
//! and WebKitGTK's does not read one at all.
//!
//! Only `127.0.0.1`, on a port the system picks. A file is served only
//! under a name of 64 random hex characters that this process gave to that
//! one file (`MediaServer::url`); anything else is 404. GET and HEAD, with
//! one byte range (`Range: bytes=…`), read from the disk a piece at a time.
//! One request for each connection.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// Names kept at once; the oldest is forgotten first.
const MAX_FILES: usize = 1024;
/// A request head longer than this is refused.
const MAX_HEAD: usize = 16 * 1024;

#[derive(Clone)]
struct Served {
    path: PathBuf,
    mime: String,
}

#[derive(Default)]
struct Names {
    by_token: HashMap<String, Served>,
    by_path: HashMap<PathBuf, String>,
    order: VecDeque<String>,
}

/// The server and the files it may give out.
#[derive(Clone, Default)]
pub struct MediaServer {
    names: Arc<Mutex<Names>>,
    port: Arc<tokio::sync::OnceCell<u16>>,
}

fn token() -> Option<String> {
    let mut bytes = [0u8; 32];
    rustls::crypto::aws_lc_rs::default_provider().secure_random.fill(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

impl MediaServer {
    /// The address the webview reads `path` at, served as `mime`; the
    /// server starts with the first one. `None` when it cannot listen.
    pub async fn url(&self, path: &Path, mime: &str) -> Option<String> {
        let port = *self.port.get_or_try_init(|| self.listen()).await.ok()?;
        let token = {
            let mut n = self.names.lock().unwrap();
            match n.by_path.get(path).cloned() {
                Some(t) => {
                    if let Some(s) = n.by_token.get_mut(&t) {
                        s.mime = mime.to_string();
                    }
                    t
                }
                None => {
                    let t = token()?;
                    n.by_token.insert(t.clone(), Served { path: path.to_path_buf(), mime: mime.to_string() });
                    n.by_path.insert(path.to_path_buf(), t.clone());
                    n.order.push_back(t.clone());
                    while n.order.len() > MAX_FILES {
                        if let Some(old) = n.order.pop_front() {
                            if let Some(s) = n.by_token.remove(&old) {
                                n.by_path.remove(&s.path);
                            }
                        }
                    }
                    t
                }
            }
        };
        Some(format!("http://127.0.0.1:{port}/{token}"))
    }

    async fn listen(&self) -> std::io::Result<u16> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let names = self.names.clone();
        tauri::async_runtime::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let names = names.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = serve(stream, names).await;
                });
            }
        });
        Ok(port)
    }
}

/// `bytes=a-b`, `bytes=a-`, `bytes=-n` of a file of `len` bytes, as an
/// inclusive range; `Err` when it cannot be satisfied, `Ok(None)` for no
/// usable range (the whole file).
fn range(header: Option<&str>, len: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else { return Ok(None) };
    if spec.contains(',') {
        return Ok(None); // several ranges: the whole file
    }
    let (a, b) = spec.split_once('-').ok_or(())?;
    let (a, b) = (a.trim(), b.trim());
    if len == 0 {
        return Err(());
    }
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        (len.saturating_sub(n), len - 1)
    } else {
        let start: u64 = a.parse().map_err(|_| ())?;
        let end = if b.is_empty() { len - 1 } else { b.parse::<u64>().map_err(|_| ())?.min(len - 1) };
        (start, end)
    };
    if start > end || start >= len {
        return Err(());
    }
    Ok(Some((start, end)))
}

async fn serve(stream: TcpStream, names: Arc<Mutex<Names>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut head = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        head.push_str(&line);
        if head.len() > MAX_HEAD {
            return reply(reader.get_mut(), "431 Request Header Fields Too Large", &[], true).await;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
    }
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or_default().split_whitespace();
    let (method, target) = (first.next().unwrap_or_default(), first.next().unwrap_or_default());
    let head_only = method == "HEAD";
    if method != "GET" && !head_only {
        return reply(reader.get_mut(), "405 Method Not Allowed", &[], true).await;
    }
    let range_header = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("range"))
        .map(|(_, v)| v.trim().to_string());
    let token = target.trim_start_matches('/').split(['?', '#', '/']).next().unwrap_or_default();
    let served = names.lock().unwrap().by_token.get(token).cloned();
    let Some(served) = served else {
        return reply(reader.get_mut(), "404 Not Found", &[], true).await;
    };
    let Ok(mut file) = tokio::fs::File::open(&served.path).await else {
        return reply(reader.get_mut(), "404 Not Found", &[], true).await;
    };
    let len = file.metadata().await?.len();
    let stream = reader.get_mut();
    let common = [
        ("Content-Type", served.mime.clone()),
        ("Accept-Ranges", "bytes".to_string()),
        ("Access-Control-Allow-Origin", "*".to_string()),
        ("Cache-Control", "no-store".to_string()),
    ];
    let (status, start, end) = match range(range_header.as_deref(), len) {
        Ok(Some((s, e))) => ("206 Partial Content", s, e),
        Ok(None) => ("200 OK", 0, len.saturating_sub(1)),
        Err(()) => {
            let h = [("Content-Range", format!("bytes */{len}"))];
            return reply(stream, "416 Range Not Satisfiable", &h, true).await;
        }
    };
    let count = if len == 0 { 0 } else { end - start + 1 };
    let mut headers: Vec<(&str, String)> = common.to_vec();
    headers.push(("Content-Length", count.to_string()));
    if status.starts_with("206") {
        headers.push(("Content-Range", format!("bytes {start}-{end}/{len}")));
    }
    reply(stream, status, &headers, false).await?;
    if head_only || count == 0 {
        return stream.shutdown().await;
    }
    file.seek(std::io::SeekFrom::Start(start)).await?;
    tokio::io::copy(&mut file.take(count), stream).await?;
    stream.shutdown().await
}

/// The status line and the headers; `end`: no body follows.
async fn reply(stream: &mut TcpStream, status: &str, headers: &[(&str, String)], end: bool) -> std::io::Result<()> {
    let mut out = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    if end {
        out.push_str("Content-Length: 0\r\n");
    }
    out.push_str("\r\n");
    stream.write_all(out.as_bytes()).await?;
    if end {
        stream.shutdown().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(range(None, 100), Ok(None));
        assert_eq!(range(Some("bytes=0-"), 100), Ok(Some((0, 99))));
        assert_eq!(range(Some("bytes=10-19"), 100), Ok(Some((10, 19))));
        assert_eq!(range(Some("bytes=90-500"), 100), Ok(Some((90, 99))));
        assert_eq!(range(Some("bytes=-10"), 100), Ok(Some((90, 99))));
        assert_eq!(range(Some("bytes=0-1,5-6"), 100), Ok(None));
        assert_eq!(range(Some("bytes=100-"), 100), Err(()));
        assert_eq!(range(Some("bytes=5-1"), 100), Err(()));
        assert_eq!(range(Some("bytes=x-"), 100), Err(()));
    }

    async fn get(url: &str, extra: &str) -> (String, Vec<u8>) {
        let rest = url.strip_prefix("http://").unwrap();
        let (host, path) = rest.split_once('/').unwrap();
        let mut s = TcpStream::connect(host).await.unwrap();
        s.write_all(format!("GET /{path} HTTP/1.1\r\nHost: {host}\r\n{extra}\r\n").as_bytes()).await.unwrap();
        let mut all = Vec::new();
        s.read_to_end(&mut all).await.unwrap();
        let at = all.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (String::from_utf8_lossy(&all[..at]).into_owned(), all[at + 4..].to_vec())
    }

    /// A file is read whole or by a range under the name it was given;
    /// any other name is 404, and only the loopback listens.
    #[tokio::test]
    async fn serves_a_named_file_by_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.mp4");
        let body: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &body).unwrap();
        let server = MediaServer::default();
        let url = server.url(&path, "video/mp4").await.unwrap();
        assert!(url.starts_with("http://127.0.0.1:"), "{url}");
        assert_eq!(server.url(&path, "video/mp4").await.unwrap(), url, "one name for a file");

        let (head, got) = get(&url, "").await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(head.contains("Content-Type: video/mp4") && head.contains("Accept-Ranges: bytes"), "{head}");
        assert_eq!(got, body);

        let (head, got) = get(&url, "Range: bytes=100-199\r\n").await;
        assert!(head.starts_with("HTTP/1.1 206") && head.contains("Content-Range: bytes 100-199/10000"), "{head}");
        assert_eq!(got, body[100..200]);

        let (head, _) = get(&url, "Range: bytes=20000-\r\n").await;
        assert!(head.starts_with("HTTP/1.1 416"), "{head}");

        let other = format!("{}/{}", url.rsplit_once('/').unwrap().0, "0".repeat(64));
        let (head, got) = get(&other, "").await;
        assert!(head.starts_with("HTTP/1.1 404") && got.is_empty(), "{head}");
    }
}

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
//!
//! The small pictures of the phone's gallery in the sheet of the paperclip
//! (`MediaServer::thumb_url`) take names of a pool of their own, with its
//! own bound: scrolling through thousands of photos forgets only older
//! thumbnails, never the name of a video of a chat that plays and seeks.
//! That pool forgets the thumbnail handed out longest ago (a name handed
//! out again is the newest), so the page, which keeps fewer addresses than
//! the pool holds (`THUMB_URLS` of attach/gallery.svelte.ts), never holds
//! one forgotten here. A thumbnail of an item the app may no longer read
//! loses its name (`forget_thumb`), and all of them go when the page reads
//! the gallery anew (`forget_thumbs`, the app came back to the screen).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// Names kept at once for the files of chats; the oldest is forgotten first.
const MAX_FILES: usize = 1024;
/// Names kept at once for the thumbnails of the gallery: a few screens of
/// a grid, scrolled back and forth. The page keeps fewer (`THUMB_URLS` = 384).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const MAX_THUMBS: usize = 512;
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

impl Names {
    /// The name of `path` (served as `mime`), given anew when it has none;
    /// past `max` names the oldest goes. `renew`: a name handed out again
    /// is the newest (the pool of thumbnails); else the first hand-out
    /// counts (the files of chats).
    fn name(&mut self, path: &Path, mime: &str, max: usize, renew: bool) -> Option<String> {
        if let Some(t) = self.by_path.get(path).cloned() {
            if let Some(s) = self.by_token.get_mut(&t) {
                s.mime = mime.to_string();
            }
            if renew {
                self.order.retain(|o| o != &t);
                self.order.push_back(t.clone());
            }
            return Some(t);
        }
        let t = token()?;
        self.by_token.insert(t.clone(), Served { path: path.to_path_buf(), mime: mime.to_string() });
        self.by_path.insert(path.to_path_buf(), t.clone());
        self.order.push_back(t.clone());
        while self.order.len() > max {
            if let Some(old) = self.order.pop_front() {
                if let Some(s) = self.by_token.remove(&old) {
                    self.by_path.remove(&s.path);
                }
            }
        }
        Some(t)
    }

    /// The name of `path` goes: its address answers 404 from now on.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    fn forget(&mut self, path: &Path) {
        if let Some(t) = self.by_path.remove(path) {
            self.by_token.remove(&t);
            self.order.retain(|o| o != &t);
        }
    }

    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    fn clear(&mut self) {
        *self = Names::default();
    }
}

/// The names given out: of the files of chats, and of the thumbnails of the gallery.
#[derive(Default)]
struct Pools {
    files: Names,
    thumbs: Names,
}

impl Pools {
    fn served(&self, token: &str) -> Option<Served> {
        self.files.by_token.get(token).or_else(|| self.thumbs.by_token.get(token)).cloned()
    }
}

/// The server and the files it may give out.
#[derive(Clone, Default)]
pub struct MediaServer {
    names: Arc<Mutex<Pools>>,
    port: Arc<tokio::sync::OnceCell<u16>>,
}

fn token() -> Option<String> {
    let mut bytes = [0u8; 32];
    rustls::crypto::aws_lc_rs::default_provider().secure_random.fill(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Lent to the channels of communities (veydan-shell `MediaUrls`): their
/// files play and seek as a chat's do.
impl veydan_shell::seams::MediaUrls for MediaServer {
    fn url<'a>(&'a self, path: &'a Path, mime: &'a str) -> veydan_shell::seams::SeamFuture<'a, Option<String>> {
        Box::pin(async move { Ok(MediaServer::url(self, path, mime).await) })
    }
}

impl MediaServer {
    /// The address the webview reads `path` at, served as `mime`; the
    /// server starts with the first one. `None` when it cannot listen.
    pub async fn url(&self, path: &Path, mime: &str) -> Option<String> {
        let port = *self.port.get_or_try_init(|| self.listen()).await.ok()?;
        let token = self.names.lock().unwrap().files.name(path, mime, MAX_FILES, false)?;
        Some(format!("http://127.0.0.1:{port}/{token}"))
    }

    /// The address of a thumbnail of the gallery (a JPEG the plugin wrote
    /// into the app's cache), named from the pool of thumbnails.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub async fn thumb_url(&self, path: &Path) -> Option<String> {
        let port = *self.port.get_or_try_init(|| self.listen()).await.ok()?;
        let token = self.names.lock().unwrap().thumbs.name(path, "image/jpeg", MAX_THUMBS, true)?;
        Some(format!("http://127.0.0.1:{port}/{token}"))
    }

    /// The thumbnails at `paths` lose their names (the app may no longer
    /// read their items): what was handed out for them answers 404.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub fn forget_thumb(&self, paths: &[PathBuf]) {
        let mut names = self.names.lock().unwrap();
        for path in paths {
            names.thumbs.forget(path);
        }
    }

    /// Every thumbnail handed out loses its name (the page reads the
    /// gallery anew: the access may have changed while the app was away).
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub fn forget_thumbs(&self) {
        self.names.lock().unwrap().thumbs.clear();
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

async fn serve(stream: TcpStream, names: Arc<Mutex<Pools>>) -> std::io::Result<()> {
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
    let served = names.lock().unwrap().served(token);
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

    /// Scrolling through more thumbnails than their pool holds forgets
    /// the oldest thumbnails only: the name of a video of a chat still
    /// serves its ranges (ROO-110, 1b-2), and a forgotten thumbnail is 404.
    #[tokio::test]
    async fn thumbnails_do_not_push_out_the_files_of_chats() {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("v.mp4");
        std::fs::write(&video, vec![7u8; 4096]).unwrap();
        let server = MediaServer::default();
        let url = server.url(&video, "video/mp4").await.unwrap();

        let thumb = dir.path().join("t.jpg");
        std::fs::write(&thumb, b"\xFF\xD8thumb").unwrap();
        let first = server.thumb_url(&thumb).await.unwrap();
        assert_eq!(server.thumb_url(&thumb).await.unwrap(), first, "one name for a thumbnail");
        let (head, got) = get(&first, "").await;
        assert!(head.starts_with("HTTP/1.1 200") && head.contains("Content-Type: image/jpeg"), "{head}");
        assert_eq!(got, b"\xFF\xD8thumb");

        // Thousands of photos scrolled past, each a name of the pool of thumbnails.
        for i in 0..(MAX_FILES + MAX_THUMBS) * 4 {
            server.thumb_url(&dir.path().join(format!("{i}.jpg"))).await.unwrap();
        }
        let (head, got) = get(&url, "Range: bytes=100-199\r\n").await;
        assert!(head.starts_with("HTTP/1.1 206"), "the video still seeks: {head}");
        assert_eq!(got.len(), 100);
        let (head, _) = get(&first, "").await;
        assert!(head.starts_with("HTTP/1.1 404"), "the oldest thumbnail is forgotten: {head}");
        let n = server.names.lock().unwrap();
        assert_eq!((n.files.order.len(), n.thumbs.order.len()), (1, MAX_THUMBS));
    }

    /// The pool of thumbnails forgets the one handed out longest ago: the
    /// page scrolls forward past the pool and back, and every address it
    /// keeps (fewer than the pool) still answers; a thumbnail asked again
    /// is the newest. One forgotten (no access) or all of them (the page
    /// reads anew) answer 404 at once.
    #[tokio::test]
    async fn the_pool_of_thumbnails_keeps_what_was_handed_out_last() {
        let dir = tempfile::tempdir().unwrap();
        let server = MediaServer::default();
        let path = |i: usize| {
            let p = dir.path().join(format!("{i}.jpg"));
            std::fs::write(&p, b"\xFF\xD8").unwrap();
            p
        };
        // What the page keeps: its newest addresses, fewer than the pool.
        const KEPT: usize = 384;
        let mut kept: std::collections::VecDeque<(usize, String)> = Default::default();
        let hand = |i: usize, url: String, kept: &mut std::collections::VecDeque<(usize, String)>| {
            kept.retain(|(j, _)| *j != i);
            kept.push_back((i, url));
            while kept.len() > KEPT {
                kept.pop_front();
            }
        };
        // Forward through 1 500 tiles, then back to the first, a screen of 24 at a time.
        let order: Vec<usize> = (0..1500).chain((0..1500).rev()).collect();
        for screen in order.chunks(24) {
            for &i in screen {
                // The page asks only what it does not keep.
                if kept.iter().any(|(j, _)| *j == i) {
                    continue;
                }
                let url = server.thumb_url(&path(i)).await.unwrap();
                hand(i, url, &mut kept);
            }
            for (_, url) in kept.iter().rev().take(24) {
                assert!(get(url, "").await.0.starts_with("HTTP/1.1 200"));
            }
        }
        for (i, url) in &kept {
            assert!(get(url, "").await.0.starts_with("HTTP/1.1 200"), "tile {i} kept by the page is served");
        }

        // Asked again, the oldest is the newest and outlives those after it.
        let a = server.thumb_url(&path(5000)).await.unwrap();
        for i in 6000..6000 + MAX_THUMBS - 1 {
            server.thumb_url(&path(i)).await.unwrap();
        }
        assert_eq!(server.thumb_url(&path(5000)).await.unwrap(), a);
        server.thumb_url(&path(7000)).await.unwrap();
        assert!(get(&a, "").await.0.starts_with("HTTP/1.1 200"), "renewed, it stays");

        server.forget_thumb(&[path(5000)]);
        assert!(get(&a, "").await.0.starts_with("HTTP/1.1 404"), "an item out of the access");
        let b = server.thumb_url(&path(7000)).await.unwrap();
        server.forget_thumbs();
        assert!(get(&b, "").await.0.starts_with("HTTP/1.1 404"), "all of them, on a new read");
        assert_ne!(server.thumb_url(&path(7000)).await.unwrap(), b, "a new name, not the old one");
    }
}

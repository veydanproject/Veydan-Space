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
//! A WebM whose video says it has alpha though it carries none (a circle
//! or a video of Chrome's recorder, messenger-media `webm`) is sent with
//! that mark put to 0, so WebKitGTK plays it; the file on the disk stays
//! as it is. Its marks are found on the very file a request opened and
//! kept with the version of that file (its length, times and, on Unix,
//! its inode): another file put under the same path, or the same file
//! changed, has them found anew, and when that fails or the file changes
//! while read it goes as it is. They are first found when the file is
//! named, so a request seldom waits for them.
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
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
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
/// The type whose files go with their false alpha put to 0.
const WEBM: &str = "video/webm";

#[derive(Clone)]
struct Served {
    path: PathBuf,
    mime: String,
    /// The marks of a WebM found last and the version they were found of.
    marks: Marks,
}

/// The bytes of a version of a file sent as 0 (`webm::alpha_marks_of_open`).
type Marks = Arc<Mutex<Option<(Version, Arc<Vec<Range<u64>>>)>>>;

/// What tells one state of a file from another: a file put under the same
/// path is another inode, one written anew has other times.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Version {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    node: (u64, u64, i64, i64),
}

impl Version {
    fn of(file: &std::fs::File) -> Option<Version> {
        let m = file.metadata().ok()?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Some(Version {
            len: m.len(),
            modified: m.modified().ok(),
            #[cfg(unix)]
            node: (m.dev(), m.ino(), m.ctime(), m.ctime_nsec()),
        })
    }
}

/// The marks of the open `file`, at `version`: those kept in `known`
/// when they were found of that version, else found now on this very
/// file and kept. None when it changed while read.
fn marks_of(file: &std::fs::File, version: &Version, known: &Marks) -> Arc<Vec<Range<u64>>> {
    if let Some((v, zero)) = &*known.lock().unwrap() {
        if v == version {
            return zero.clone();
        }
    }
    let zero = Arc::new(messenger_media::webm::alpha_marks_of_open(file, version.len));
    if Version::of(file).as_ref() != Some(version) {
        return Arc::default();
    }
    *known.lock().unwrap() = Some((version.clone(), zero.clone()));
    zero
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
        self.by_token.insert(t.clone(), Served { path: path.to_path_buf(), mime: mime.to_string(), marks: Marks::default() });
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
        let (token, marks) = {
            let mut names = self.names.lock().unwrap();
            let token = names.files.name(path, mime, MAX_FILES, false)?;
            let marks = names.files.by_token.get(&token).map(|s| s.marks.clone());
            (token, marks)
        };
        if let (WEBM, Some(marks)) = (mime, marks) {
            let path = path.to_path_buf();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                let Ok(file) = std::fs::File::open(&path) else { return };
                if let Some(version) = Version::of(&file) {
                    marks_of(&file, &version, &marks);
                }
            })
            .await;
        }
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
    // The marks are of the file this request reads, whatever lay under its path before.
    let (path, webm, marks) = (served.path.clone(), served.mime == WEBM, served.marks.clone());
    let opened = tauri::async_runtime::spawn_blocking(move || {
        let file = std::fs::File::open(&path).ok()?;
        let version = Version::of(&file)?;
        let zero = if webm { marks_of(&file, &version, &marks) } else { Arc::default() };
        Some((file, version.len, zero))
    })
    .await;
    let Ok(Some((file, len, zero))) = opened else {
        return reply(reader.get_mut(), "404 Not Found", &[], true).await;
    };
    let mut file = tokio::fs::File::from_std(file);
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
    if zero.is_empty() {
        tokio::io::copy(&mut file.take(count), stream).await?;
        return stream.shutdown().await;
    }
    let mut piece = vec![0; 64 * 1024];
    let (mut at, end) = (start, start + count);
    while at < end {
        let want = (end - at).min(piece.len() as u64) as usize;
        let n = file.read(&mut piece[..want]).await?;
        if n == 0 {
            break;
        }
        put_zero(&mut piece[..n], at, &zero);
        stream.write_all(&piece[..n]).await?;
        at += n as u64;
    }
    stream.shutdown().await
}

/// The bytes of `piece`, read at `at` of its file, that lie in `zero` put to 0.
fn put_zero(piece: &mut [u8], at: u64, zero: &[Range<u64>]) {
    let end = at + piece.len() as u64;
    for z in zero {
        let (a, b) = (z.start.max(at), z.end.min(end));
        if a < b {
            piece[(a - at) as usize..(b - at) as usize].fill(0);
        }
    }
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

    /// A WebM whose video says it has alpha and has none goes with that
    /// mark as 0, whole and by a range; the file on the disk is not changed.
    #[tokio::test]
    async fn a_webm_goes_without_its_false_alpha() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.webm");
        // Segment > Tracks > TrackEntry > Video > PixelWidth 480, AlphaMode 1 (the last byte).
        let hex = "18538067911654ae6b8cae8ae088b08201e053c08101";
        let body: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        std::fs::write(&path, &body).unwrap();
        let last = body.len() - 1;
        let mut want = body.clone();
        want[last] = 0;
        let server = MediaServer::default();

        let mp4 = server.url(&path, "video/mp4").await.unwrap();
        assert_eq!(get(&mp4, "").await.1, body, "not a WebM, it goes as it is");
        let url = server.url(&path, "video/webm").await.unwrap();
        assert_eq!(url, mp4, "one name for a file");
        let (head, got) = get(&url, "").await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert_eq!(got, want);
        let (_, got) = get(&url, &format!("Range: bytes={}-\r\n", last - 3)).await;
        assert_eq!(got, want[last - 3..]);
        let (_, got) = get(&url, "Range: bytes=0-3\r\n").await;
        assert_eq!(got, body[..4]);
        assert_eq!(std::fs::read(&path).unwrap(), body, "the file stays");

        // The file of a WebM with alpha (here: one not read through) goes as it is.
        let other = dir.path().join("a.webm");
        std::fs::write(&other, &body[..last]).unwrap();
        let url = server.url(&other, "video/webm").await.unwrap();
        assert_eq!(get(&url, "").await.1, body[..last]);
    }

    /// Another file put under the path of a WebM named before, or the
    /// same file written anew, goes by its own marks: whether it is named
    /// again or not, of the same length, and a WebM with alpha as it is.
    #[tokio::test]
    async fn a_file_put_under_the_same_path_goes_by_its_own_marks() {
        let bytes = |hex: &str| -> Vec<u8> { (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect() };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.webm");
        let put = |body: &[u8]| {
            let tmp = dir.path().join("tmp");
            std::fs::write(&tmp, body).unwrap();
            std::fs::rename(&tmp, &path).unwrap();
        };
        // Segment > Tracks > TrackEntry > Video > PixelWidth 480, AlphaMode 1 (the last byte).
        let body = bytes("18538067911654ae6b8cae8ae088b08201e053c08101");
        let last = body.len() - 1;
        let mut want = body.clone();
        want[last] = 0;
        put(&body);
        let server = MediaServer::default();
        let url = server.url(&path, WEBM).await.unwrap();
        assert_eq!(get(&url, "").await.1, want);

        // Put under the path after the name was given, before a request; of the same length.
        let other: Vec<u8> = (0..body.len() as u8).map(|i| 0x40 + i).collect();
        put(&other);
        assert_eq!(get(&url, "").await.1, other, "not a mark of the file before");
        assert_eq!(get(&url, &format!("Range: bytes={last}-\r\n")).await.1, other[last..]);
        put(&body);
        assert_eq!(get(&url, "").await.1, want, "found anew");

        // Written anew in place, the same length, and named again.
        std::fs::write(&path, &other).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1)).unwrap();
        drop(file);
        assert_eq!(server.url(&path, WEBM).await.unwrap(), url, "one name for a path");
        assert_eq!(get(&url, "").await.1, other);

        // A WebM whose cluster carries alpha (BlockAddID 1), the same head as one without.
        let head = "1853806701ffffffffffffff1654ae6b8cae8ae088b08201e053c08101";
        let cluster = "1f43b67501ffffffffffffffe78100";
        let plain = bytes(&format!("{head}{cluster}a386810000008000"));
        let mark = bytes(head).len() - 1;
        let mut cleared = plain.clone();
        cleared[mark] = 0;
        put(&plain);
        assert_eq!(server.url(&path, WEBM).await.unwrap(), url);
        assert_eq!(get(&url, "").await.1, cleared, "a false alpha");
        let alpha = bytes(&format!("{head}{cluster}a096a185810000008075a18ca68aee8101a585616c706861"));
        put(&alpha);
        assert_eq!(get(&url, "").await.1, alpha, "its alpha stays");
        assert_eq!(get(&url, &format!("Range: bytes={mark}-{mark}\r\n")).await.1, [1]);
        assert_eq!(std::fs::read(&path).unwrap(), alpha, "the file stays");
    }

    #[test]
    fn zeros_fall_on_their_bytes_only() {
        let mut p = [9u8; 10];
        put_zero(&mut p, 100, &[95..102, 105..106, 110..120, 50..60]);
        assert_eq!(p, [0, 0, 9, 9, 9, 0, 9, 9, 9, 9]);
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

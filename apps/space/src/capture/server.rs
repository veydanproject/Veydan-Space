// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Local socket server inside the app. Line protocol: one JSON request line in,
//! one JSON response line out, then the connection closes. It runs while the
//! notes are switched on: `start` gives the task, aborting it closes the socket.
//!
//! The line is the envelope `{"token":"<hex>","req":<request>}` (`envelope_head`):
//! the server reads its head, compares the token and only then parses the
//! request. A connection without the token, or with another one, is closed
//! without an answer. On Unix the peer must also be the user who owns the
//! app data folder.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use veydan_core::Core;
use veydan_lock::Lock;
use veydan_notes::capture::protocol::{CaptureRequest, CaptureResponse};

/// How long a client has to show the token.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);

/// Equal bytes, compared in a time that does not depend on where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn serve_connection<S, H, F>(stream: S, head: &[u8], handle: H)
where
    S: AsyncRead + AsyncWrite + Unpin,
    H: FnOnce(CaptureRequest) -> F,
    F: Future<Output = CaptureResponse>,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let mut got = vec![0u8; head.len()];
    match tokio::time::timeout(HEAD_TIMEOUT, reader.read_exact(&mut got)).await {
        Ok(Ok(_)) if same(&got, head) => {}
        // No token, another token, or nothing in time: closed without an answer.
        _ => return,
    }
    let mut line = String::new();
    if reader.read_line(&mut line).await.is_err() {
        return;
    }
    let response = match line.trim_end().strip_suffix('}') {
        Some(body) => match serde_json::from_str::<CaptureRequest>(body) {
            Ok(req) => handle(req).await,
            Err(e) => CaptureResponse::err(format!("Bad request: {e}")),
        },
        None => CaptureResponse::err("Bad request: the envelope is not closed"),
    };
    let mut out = serde_json::to_vec(&response).unwrap_or_default();
    out.push(b'\n');
    let _ = writer.write_all(&out).await;
    let _ = writer.shutdown().await;
}

/// One connection answered by the notes of `app`.
async fn serve_app<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    head: Arc<[u8]>,
    app: tauri::AppHandle,
) {
    serve_connection(stream, &head, move |req| async move {
        let locked = app.state::<Lock>().is_locked().await;
        let rules = super::rules::load(&app.state::<Core>().db).await;
        veydan_notes::capture::handle_capture(&app, req, &rules, locked).await
    })
    .await
}

/// The bridge of the app's data folder, or why there is none.
fn bridge(app: &tauri::AppHandle) -> Option<(std::path::PathBuf, super::Bridge)> {
    let data_dir = app.state::<Core>().app_data_dir.clone();
    match super::bridge(&data_dir) {
        Ok(bridge) => Some((data_dir, bridge)),
        Err(e) => {
            eprintln!("capture: cannot read or make the secret of the bridge: {e}");
            None
        }
    }
}

/// The task of the server; aborting it drops the listener.
pub struct Server {
    task: tauri::async_runtime::JoinHandle<()>,
    /// The socket file, removed on `stop`.
    #[cfg(not(windows))]
    path: Option<String>,
}

#[cfg(not(windows))]
pub fn start(app: tauri::AppHandle) -> Server {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let bridge = bridge(&app);
    let path = bridge.as_ref().map(|(_, b)| b.endpoint.clone());
    if let Some(path) = &path {
        let _ = std::fs::remove_file(path);
    }
    // Bind inside the runtime: UnixListener needs a tokio reactor
    let task = tauri::async_runtime::spawn(async move {
        let Some((data_dir, bridge)) = bridge else {
            return;
        };
        let path = bridge.endpoint;
        // Only the user who owns the app data folder may talk to the bridge.
        let owner = match std::fs::metadata(&data_dir) {
            Ok(m) => m.uid(),
            Err(e) => {
                eprintln!("capture: cannot read {}: {e}", data_dir.display());
                return;
            }
        };
        let listener = match tokio::net::UnixListener::bind(&path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("capture: cannot bind {path}: {e}");
                return;
            }
        };
        if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
            eprintln!("capture: cannot restrict {path}: {e}");
            return;
        }
        let head: Arc<[u8]> = super::envelope_head(&bridge.token).into_bytes().into();
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    match stream.peer_cred() {
                        Ok(cred) if cred.uid() == owner => {}
                        // Another user: closed without an answer.
                        _ => continue,
                    }
                    let app = app.clone();
                    tauri::async_runtime::spawn(serve_app(stream, head.clone(), app));
                }
                Err(e) => {
                    eprintln!("capture: accept failed: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }
    });
    Server { task, path }
}

/// The server's task is aborted: the socket file goes with it.
#[cfg(not(windows))]
pub fn stop(server: Server) {
    server.task.abort();
    if let Some(path) = server.path {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(windows)]
pub fn start(app: tauri::AppHandle) -> Server {
    use tokio::net::windows::named_pipe::ServerOptions;
    let bridge = bridge(&app);
    let task = tauri::async_runtime::spawn(async move {
        let Some((_, bridge)) = bridge else {
            return;
        };
        let name = bridge.endpoint;
        let head: Arc<[u8]> = super::envelope_head(&bridge.token).into_bytes().into();
        // The first instance must be ours: a pipe of this name that already
        // exists belongs to someone else. While the server runs, one instance
        // always waits (the next is made before the connected one is served),
        // so the name is never free for another process to take.
        let mut server = match ServerOptions::new().first_pipe_instance(true).create(&name) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("capture: cannot create pipe {name}: {e}");
                return;
            }
        };
        loop {
            if let Err(e) = server.connect().await {
                eprintln!("capture: pipe connect failed: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                continue;
            }
            let connected = server;
            server = match ServerOptions::new().create(&name) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("capture: cannot recreate pipe {name}: {e}");
                    return;
                }
            };
            tauri::async_runtime::spawn(serve_app(connected, head.clone(), app.clone()));
        }
    });
    Server { task }
}

/// The server's task is aborted: the pipe closes with it.
#[cfg(windows)]
pub fn stop(server: Server) {
    server.task.abort();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hex of few kinds of digits: a test value, not a key (the secret scan reads it so).
    const TOKEN: &str = "0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a";
    const REQUEST: &str = r#"{"kind":"list_notes","profile_id":"p1"}"#;

    /// Sends `line` to a connection served with `TOKEN`; the answer, if any.
    async fn exchange(line: &str) -> (Option<CaptureResponse>, bool) {
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let head = crate::capture::envelope_head(TOKEN).into_bytes();
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = called.clone();
        let serve = tokio::spawn(async move {
            serve_connection(server, &head, move |req| async move {
                seen.store(true, std::sync::atomic::Ordering::SeqCst);
                assert_eq!(req.kind, "list_notes");
                CaptureResponse::done()
            })
            .await
        });
        client.write_all(line.as_bytes()).await.unwrap();
        // The end of what the client sends: a short line ends the wait for the head.
        client.shutdown().await.unwrap();
        let mut out = String::new();
        let _ = client.read_to_string(&mut out).await;
        serve.await.unwrap();
        let response = if out.is_empty() {
            None
        } else {
            Some(serde_json::from_str(out.trim()).unwrap())
        };
        (response, called.load(std::sync::atomic::Ordering::SeqCst))
    }

    #[tokio::test]
    async fn the_token_opens_the_bridge() {
        let line = format!("{}{REQUEST}}}\n", crate::capture::envelope_head(TOKEN));
        let (response, called) = exchange(&line).await;
        assert!(response.expect("an answer").ok);
        assert!(called);
    }

    #[tokio::test]
    async fn a_request_without_the_token_is_closed_unanswered() {
        // The old protocol: the bare request.
        let (response, called) = exchange(&format!("{REQUEST}\n")).await;
        assert!(response.is_none());
        assert!(!called);
    }

    #[tokio::test]
    async fn another_token_is_closed_unanswered() {
        let other = "ffeeddccbbaa99887766554433221100";
        let line = format!("{}{REQUEST}}}\n", crate::capture::envelope_head(other));
        let (response, called) = exchange(&line).await;
        assert!(response.is_none());
        assert!(!called);
    }

    #[tokio::test]
    async fn a_short_line_is_closed_unanswered() {
        let (response, called) = exchange("{\"token\":\"00\"}\n").await;
        assert!(response.is_none());
        assert!(!called);
    }

    #[tokio::test]
    async fn a_bad_request_behind_the_token_is_answered_with_an_error() {
        let line = format!("{}{{\"kind\":1}}}}\n", crate::capture::envelope_head(TOKEN));
        let (response, called) = exchange(&line).await;
        let response = response.expect("an answer");
        assert!(!response.ok);
        assert!(!called);
    }

    #[test]
    fn same_compares_every_byte() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"ab"));
    }
}

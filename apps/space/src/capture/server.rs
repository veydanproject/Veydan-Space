// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Local socket server inside the app. Line protocol: one JSON request line in,
//! one JSON response line out, then the connection closes. It runs while the
//! notes are switched on: `start` gives the task, aborting it closes the socket.

use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use veydan_core::Core;
use veydan_lock::Lock;
use veydan_notes::capture::protocol::{CaptureRequest, CaptureResponse};

async fn serve_connection<S: AsyncRead + AsyncWrite + Unpin>(stream: S, app: tauri::AppHandle) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut line = String::new();
    if BufReader::new(reader).read_line(&mut line).await.is_err() {
        return;
    }
    let response = match serde_json::from_str::<CaptureRequest>(line.trim()) {
        Ok(req) => {
            let locked = app.state::<Lock>().is_locked().await;
            let rules = super::rules::load(&app.state::<Core>().db).await;
            veydan_notes::capture::handle_capture(&app, req, &rules, locked).await
        }
        Err(e) => CaptureResponse::err(format!("Bad request: {e}")),
    };
    let mut out = serde_json::to_vec(&response).unwrap_or_default();
    out.push(b'\n');
    let _ = writer.write_all(&out).await;
    let _ = writer.shutdown().await;
}

/// The task of the server; aborting it drops the listener.
pub type Server = tauri::async_runtime::JoinHandle<()>;

#[cfg(not(windows))]
pub fn start(app: tauri::AppHandle) -> Server {
    let path = super::ipc_endpoint();
    let _ = std::fs::remove_file(&path);
    // Bind inside the runtime: UnixListener needs a tokio reactor
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::UnixListener::bind(&path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("capture: cannot bind {path}: {e}");
                return;
            }
        };
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(serve_connection(stream, app));
                }
                Err(e) => {
                    eprintln!("capture: accept failed: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }
    })
}

/// The server's task is aborted: the socket file goes with it.
#[cfg(not(windows))]
pub fn stop(server: Server) {
    server.abort();
    let _ = std::fs::remove_file(super::ipc_endpoint());
}

#[cfg(windows)]
pub fn start(app: tauri::AppHandle) -> Server {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = super::ipc_endpoint();
    tauri::async_runtime::spawn(async move {
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
            tauri::async_runtime::spawn(serve_connection(connected, app.clone()));
        }
    })
}

/// The server's task is aborted: the pipe closes with it.
#[cfg(windows)]
pub fn stop(server: Server) {
    server.abort();
}

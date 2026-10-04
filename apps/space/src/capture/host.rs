// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Native messaging host mode: frames on stdin/stdout (4-byte LE length + JSON),
//! each request relayed over the local socket to the running app.
//! Plain std I/O: this runs before (instead of) the Tauri runtime.

use std::io::{BufRead, BufReader, Read, Write};
use veydan_notes::capture::protocol::CaptureResponse;

// Article captures carry base64 images (up to ~10MB raw)
const MAX_FRAME: usize = 64 * 1024 * 1024;

fn read_frame(stdin: &mut impl Read) -> Option<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stdin.read_exact(&mut len_buf).ok()?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 || len > MAX_FRAME {
        return None;
    }
    let mut buf = vec![0u8; len];
    stdin.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn write_frame(stdout: &mut impl Write, payload: &[u8]) {
    let _ = stdout.write_all(&(payload.len() as u32).to_le_bytes());
    let _ = stdout.write_all(payload);
    let _ = stdout.flush();
}

#[cfg(not(windows))]
fn connect() -> std::io::Result<Box<dyn ReadWrite>> {
    let s = std::os::unix::net::UnixStream::connect(super::host_endpoint())?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(15)))?;
    Ok(Box::new(s))
}

#[cfg(windows)]
fn connect() -> std::io::Result<Box<dyn ReadWrite>> {
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(super::host_endpoint())?;
    Ok(Box::new(f))
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

/// One request -> one newline-terminated JSON line each way.
fn relay(request: &[u8]) -> Vec<u8> {
    let mut stream = match connect() {
        Ok(s) => s,
        Err(_) => {
            // No socket: the app is not running, or its notes are switched off
            // and the socket closed with them (platform-spec 12).
            return serde_json::to_vec(&CaptureResponse::err(
                "Veydan is not running, or Notes are switched off in it",
            ))
            .unwrap_or_default();
        }
    };
    if stream.write_all(request).is_err()
        || stream.write_all(b"\n").is_err()
        || stream.flush().is_err()
    {
        return serde_json::to_vec(&CaptureResponse::err("Failed to send request to Veydan"))
            .unwrap_or_default();
    }
    let mut line = String::new();
    let mut reader = BufReader::new(stream);
    match reader.read_line(&mut line) {
        Ok(n) if n > 0 => line.trim_end().as_bytes().to_vec(),
        _ => {
            serde_json::to_vec(&CaptureResponse::err("No response from Veydan")).unwrap_or_default()
        }
    }
}

/// Serve stdin until the browser closes the pipe.
pub fn run() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    while let Some(frame) = read_frame(&mut input) {
        let response = relay(&frame);
        write_frame(&mut output, &response);
    }
}

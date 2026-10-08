// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The log of the calls on a computer: `calls.log` in the messenger's
//! data folder, with what the engine and the core of calls say of the
//! way to the node (ICE, DTLS, the data channel), of the words of
//! identity of a room and of the judgments of the seats — the lines of
//! `messenger_rtc` and `messenger_calls` (through the `log` bridge of
//! `tracing`: no subscriber of `tracing` is installed in a product) and
//! those of libwebrtc about its transports (the sink of the `libwebrtc`
//! crate writes to `log` under the target `libwebrtc`). A phone writes
//! the same to logcat by itself; a computer had nothing, and a call that
//! failed on one left no trace to read (the owner's room of 2026-10-08,
//! "Входим…" with every seat unconfirmed).
//!
//! The file is kept small: started over when it has grown past
//! [`MAX_BYTES`] at the start of the runtime, appended to otherwise, and
//! a line of libwebrtc that is not about a transport is not written.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

/// The file of the log, in the messenger's data folder.
pub const FILE_NAME: &str = "calls.log";
/// A log that has grown past this is started over at the next start.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// What of libwebrtc is kept: the transports, not every port and
/// candidate.
const LIBWEBRTC_KEPT: [&str; 18] = [
    "sctp",
    "Sctp",
    "SCTP",
    "dtls",
    "Dtls",
    "DTLS",
    "DataChannel",
    "data_channel",
    "datachannel",
    "p2p_transport_channel",
    "peer_connection.cc",
    "jsep_transport",
    "Transport",
    "transport",
    "ice_",
    "Ice",
    "ICE",
    "Failed",
];
const LIBWEBRTC_NOISE: [&str; 4] = ["(connection.cc:", "(basic_port_allocator.cc:", "(stun_port.cc:", "(port.cc:"];

struct CallsLog {
    file: Mutex<Option<File>>,
}

static LOG: CallsLog = CallsLog { file: Mutex::new(None) };

impl log::Log for CallsLog {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        let t = m.target();
        t.starts_with("messenger_rtc") || t.starts_with("messenger_calls") || t == "libwebrtc"
    }

    fn log(&self, r: &log::Record<'_>) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let t = r.target();
        let m = r.args().to_string();
        if t == "libwebrtc" && (LIBWEBRTC_NOISE.iter().any(|k| m.contains(k)) || !LIBWEBRTC_KEPT.iter().any(|k| m.contains(k))) {
            return;
        }
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = file.as_mut() {
            let _ = writeln!(f, "{} {} {t}: {m}", stamp(), r.level());
        }
    }

    fn flush(&self) {
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = file.as_mut() {
            let _ = f.flush();
        }
    }
}

/// Unix seconds and milliseconds, as the CLI of the messenger stamps
/// its lines: the logs of two devices line up.
fn stamp() -> String {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("[{}.{:03}]", ms / 1000, ms % 1000)
}

/// Open the log in `data_dir` (made if it is not there) and take the
/// `log` records of the process from now on. Nothing happens when the
/// file cannot be opened, or when another logger was installed first:
/// the messenger runs without its calls log, as before.
pub fn install(data_dir: &Path) {
    let _ = std::fs::create_dir_all(data_dir);
    let path = data_dir.join(FILE_NAME);
    let fresh = std::fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(true);
    let opened = if fresh { File::create(&path) } else { OpenOptions::new().append(true).open(&path) };
    let Ok(mut file) = opened else { return };
    let _ = writeln!(file, "{} INFO calls_log: the log of calls {}", stamp(), if fresh { "started" } else { "continues" });
    *LOG.file.lock().unwrap_or_else(|e| e.into_inner()) = Some(file);
    if log::set_logger(&LOG).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Log;

    #[test]
    fn the_lines_of_the_engine_and_of_the_transports_are_kept_and_the_rest_is_not() {
        let kept = |target: &str, msg: &str| {
            let meta = log::MetadataBuilder::new().target(target).level(log::Level::Debug).build();
            LOG.enabled(&meta)
                && !(target == "libwebrtc" && (LIBWEBRTC_NOISE.iter().any(|k| msg.contains(k)) || !LIBWEBRTC_KEPT.iter().any(|k| msg.contains(k))))
        };
        assert!(kept("messenger_rtc::session", "connection state"));
        assert!(kept("messenger_calls::group::service", "the seat is confirmed"));
        assert!(kept("libwebrtc", "(dtls_transport.cc:123): DtlsTransport[0|1|__]: DTLS handshake"));
        assert!(kept("libwebrtc", "(sctp_data_channel.cc:5): SCTP channel open"));
        assert!(!kept("libwebrtc", "(connection.cc:9): Conn[...]: a ping of a transport pair"), "the pings of every pair are noise");
        assert!(!kept("libwebrtc", "(audio_device.cc:1): a device"));
        assert!(!kept("sqlx::query", "SELECT"));
    }
}

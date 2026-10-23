// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The terminal of a phone. An Android app's stdout and stderr go to
//! /dev/null, so every line the modules write for the owner's terminal
//! (`communities: …`, `sync: …`) was lost on a phone, and a join that
//! never reached its keeper left no trace there (живой запуск 2026-10-23).
//! Both are turned into a pipe at start; a thread hands each line to
//! logcat under the tag `veydan`: `adb logcat -s veydan`.

use std::ffi::{c_char, c_int, CString};
use std::io::{BufRead, BufReader};
use std::os::fd::AsRawFd;

const TAG: &[u8] = b"veydan\0";
/// `ANDROID_LOG_INFO` of `<android/log.h>`.
const INFO: c_int = 4;
/// logcat cuts a line at about 4 KB; longer ones go in pieces.
const PIECE: usize = 3800;

extern "C" {
    fn dup2(old: c_int, new: c_int) -> c_int;
}

#[link(name = "log")]
extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

fn write(line: &[u8]) {
    for piece in line.chunks(PIECE) {
        let text: Vec<u8> = piece.iter().copied().filter(|b| *b != 0).collect();
        if let Ok(text) = CString::new(text) {
            // SAFETY: both are NUL-terminated strings that outlive the call.
            unsafe { __android_log_write(INFO, TAG.as_ptr().cast(), text.as_ptr()) };
        }
    }
}

/// Once, before anything is written: stdout and stderr to logcat. When the
/// pipe or its reader cannot be made, they stay where they were (a write to
/// a pipe nobody reads would block or fail the app).
pub(crate) fn redirect() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(redirect_once);
}

fn redirect_once() {
    let Ok((reader, writer)) = std::io::pipe() else { return };
    let spawned = std::thread::Builder::new().name("logcat".into()).spawn(move || {
        let mut lines = BufReader::new(reader);
        let mut line = Vec::new();
        loop {
            line.clear();
            match lines.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    while line.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
                        line.pop();
                    }
                    write(&line);
                }
            }
        }
    });
    if spawned.is_err() {
        return;
    }
    // SAFETY: dup2 on descriptors this process holds; the pipe's end stays
    // open as 1 and 2 after `writer` is dropped.
    unsafe {
        dup2(writer.as_raw_fd(), 1);
        dup2(writer.as_raw_fd(), 2);
    }
}

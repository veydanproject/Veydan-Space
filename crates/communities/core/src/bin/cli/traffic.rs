// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The traffic of the stand (ROO-474): the bytes this process sent to the
//! relay and took from it (events as JSON, without the frames of the
//! WebSocket and TLS) and to and from the blob hosts (the bodies of PUT
//! and GET). The quota of the test server is not known; the stand has a
//! budget, and a run stops by itself near the limit it is given.
//!
//! `COMMUNITIES_TRIAL_TRAFFIC_DIR`: every process of a run writes its
//! counts there (`<command>-<pid>.json`, every 5 s and at the end) and
//! reads the others'; `COMMUNITIES_TRIAL_TRAFFIC_LIMIT_MB`: when the sum
//! of the run reaches it, the process says so (`traffic_limit`) and ends
//! with code 3. `COMMUNITIES_TRIAL_UPLOADS`: every blob put, a JSON line
//! (host, hash, the uploader's public key), for the cleanup of the hosts.

use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static RELAY_OUT: AtomicU64 = AtomicU64::new(0);
static RELAY_IN: AtomicU64 = AtomicU64::new(0);
static MEDIA_OUT: AtomicU64 = AtomicU64::new(0);
static MEDIA_IN: AtomicU64 = AtomicU64::new(0);

/// Exit code of a process stopped by the limit.
pub const OVER_LIMIT: i32 = 3;

pub fn relay_out(n: usize) {
    RELAY_OUT.fetch_add(n as u64, Ordering::Relaxed);
}

pub fn relay_in(n: usize) {
    RELAY_IN.fetch_add(n as u64, Ordering::Relaxed);
}

pub fn media_out(n: usize) {
    MEDIA_OUT.fetch_add(n as u64, Ordering::Relaxed);
}

pub fn media_in(n: usize) {
    MEDIA_IN.fetch_add(n as u64, Ordering::Relaxed);
}

/// This process's counts.
pub fn mine() -> Value {
    let (ro, ri, mo, mi) = (RELAY_OUT.load(Ordering::Relaxed), RELAY_IN.load(Ordering::Relaxed), MEDIA_OUT.load(Ordering::Relaxed), MEDIA_IN.load(Ordering::Relaxed));
    json!({ "relay_out": ro, "relay_in": ri, "media_out": mo, "media_in": mi, "total": ro + ri + mo + mi })
}

fn dir() -> Option<PathBuf> {
    std::env::var_os("COMMUNITIES_TRIAL_TRAFFIC_DIR").filter(|d| !d.is_empty()).map(PathBuf::from)
}

fn limit_bytes() -> Option<u64> {
    std::env::var("COMMUNITIES_TRIAL_TRAFFIC_LIMIT_MB").ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|n| *n > 0).map(|mb| mb * 1_048_576)
}

/// The counts of every process of the run, summed (this one's included).
pub fn run_total() -> Value {
    let mut sum = [0u64; 4];
    let names = ["relay_out", "relay_in", "media_out", "media_in"];
    let mut processes = 0;
    if let Some(d) = dir() {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let Ok(s) = std::fs::read_to_string(e.path()) else { continue };
            let Ok(v) = serde_json::from_str::<Value>(&s) else { continue };
            processes += 1;
            for (i, n) in names.iter().enumerate() {
                sum[i] += v[n].as_u64().unwrap_or(0);
            }
        }
    }
    let total: u64 = sum.iter().sum();
    json!({ "processes": processes, "relay_mb": mb(sum[0] + sum[1]), "media_mb": mb(sum[2] + sum[3]), "total_mb": mb(total), "total": total, "limit_mb": limit_bytes().map(|b| b / 1_048_576) })
}

fn mb(b: u64) -> f64 {
    (b as f64 / 1_048_576.0 * 10.0).round() / 10.0
}

/// Writes this process's counts to the run's folder.
pub fn save(command: &str) {
    let Some(d) = dir() else { return };
    let _ = std::fs::create_dir_all(&d);
    let path = d.join(format!("{command}-{}.json", std::process::id()));
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, mine().to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Every 5 s: the counts saved, the run's sum against the limit; past it
/// the process ends.
pub fn watch(command: String) {
    tokio::spawn(async move {
        let limit = limit_bytes();
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            save(&command);
            let Some(limit) = limit else { continue };
            let total = run_total();
            if total["total"].as_u64().unwrap_or(0) >= limit {
                crate::out(json!({ "event": "traffic_limit", "command": command, "run": total, "mine": mine() }));
                std::process::exit(OVER_LIMIT);
            }
        }
    });
}

/// A blob put on a host, for the cleanup of the hosts after the run.
pub fn uploaded(host: &str, sha256: &str, uploader: &str) {
    let Some(path) = std::env::var_os("COMMUNITIES_TRIAL_UPLOADS").filter(|p| !p.is_empty()) else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", json!({ "host": host, "sha256": sha256, "uploader": uploader }));
    }
}

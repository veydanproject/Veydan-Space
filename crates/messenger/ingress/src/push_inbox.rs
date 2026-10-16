// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Events a push brought, left for the app.
//!
//! The push handler (`messenger_notify`) opens the event of a push and
//! shows it, in a process of its own and with the database read-only. So
//! that the chat holds what the notification said before a relay is
//! reached, it leaves the event here as it came: sealed, as the relay
//! keeps it. The app takes these into its ingress like any other event:
//! deduped, checked, classified and applied by the same rules. They come
//! as history (`EventSource::Sync`): the system told of them already, and
//! the app tells nothing again.
//!
//! One file per event, named by its id, written under a temporary name and
//! renamed, so that the app never reads half of one.

use messenger_core::{EventSource, RawEvent};
use std::path::Path;
use std::time::{Duration, SystemTime};

/// No more than this many wait; beyond, the relay keeps the rest.
pub const MAX_KEPT: usize = 256;
/// A temporary file older than this is what a killed handler left.
const STALE_TMP: Duration = Duration::from_secs(600);

fn is_name(stem: &str) -> bool {
    stem.len() == 64 && stem.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Leaves `raw` for the app; whether it was left (not when as many wait
/// already, nor a second time).
pub fn keep(dir: &Path, raw: &RawEvent) -> std::io::Result<bool> {
    std::fs::create_dir_all(dir)?;
    let id = raw.id.as_hex();
    let path = dir.join(format!("{id}.json"));
    if path.exists() {
        return Ok(false);
    }
    let waiting = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .count();
    if waiting >= MAX_KEPT {
        return Ok(false);
    }
    let tmp = dir.join(format!("{id}.tmp"));
    let bytes = serde_json::to_vec(raw).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(true)
}

/// Every event that waits, in the order the pushes brought them, as
/// history; what was read is gone from the folder, and so is what cannot
/// be read.
pub fn take(dir: &Path) -> Vec<RawEvent> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<(SystemTime, RawEvent)> = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
        match path.extension().and_then(|x| x.to_str()) {
            Some("json") => {}
            Some("tmp") => {
                if modified.elapsed().is_ok_and(|age| age > STALE_TMP) {
                    let _ = std::fs::remove_file(&path);
                }
                continue;
            }
            _ => continue,
        }
        let read = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<RawEvent>(&b).ok());
        let _ = std::fs::remove_file(&path);
        let Some(mut raw) = read.filter(|r| is_name(&stem) && r.id.as_hex() == stem) else { continue };
        if let EventSource::Relay { url } = raw.source {
            raw.source = EventSource::Sync { url };
        }
        found.push((modified, raw));
    }
    found.sort_by_key(|(at, _)| *at);
    found.into_iter().map(|(_, raw)| raw).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::{EventId, PubKey, RelayUrl, Timestamp};

    fn raw(id_byte: char) -> RawEvent {
        let id: String = std::iter::repeat_n(id_byte, 64).collect();
        RawEvent {
            id: EventId::parse(&id).unwrap(),
            kind: 1059,
            pubkey: PubKey::parse(&"b".repeat(64)).unwrap(),
            created_at: Timestamp(1_000),
            json: serde_json::json!({ "id": id }),
            source: EventSource::Relay { url: RelayUrl::parse("wss://r.example").unwrap() },
        }
    }

    #[test]
    fn what_is_kept_is_taken_once_as_history() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join("push-inbox");
        assert!(keep(&inbox, &raw('a')).unwrap());
        assert!(!keep(&inbox, &raw('a')).unwrap(), "the same event is left once");
        assert!(keep(&inbox, &raw('c')).unwrap());
        let taken = take(&inbox);
        assert_eq!(taken.len(), 2);
        assert!(taken.iter().all(|r| matches!(r.source, EventSource::Sync { .. })));
        assert!(take(&inbox).is_empty(), "taken means gone");
    }

    #[test]
    fn an_empty_or_missing_folder_gives_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(take(&dir.path().join("none")).is_empty());
    }

    #[test]
    fn a_file_that_is_not_an_event_of_its_name_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path();
        std::fs::write(inbox.join(format!("{}.json", "d".repeat(64))), b"not json").unwrap();
        let other = serde_json::to_vec(&raw('a')).unwrap();
        std::fs::write(inbox.join(format!("{}.json", "e".repeat(64))), other).unwrap();
        assert!(take(inbox).is_empty());
        assert_eq!(std::fs::read_dir(inbox).unwrap().count(), 0);
    }

    #[test]
    fn no_more_than_the_limit_waits() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path();
        for i in 0..MAX_KEPT {
            std::fs::write(inbox.join(format!("{i:064x}.json")), b"{}").unwrap();
        }
        assert!(!keep(inbox, &raw('a')).unwrap());
    }
}

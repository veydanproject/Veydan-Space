// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Whether another device of mine is in sight now: what a phone woken by
//! a push reads before it rings for a message (`messenger_notify`).
//!
//! The page of the app beats while it is on the screen, and a phone asleep
//! beats not at all. So a live beat of my presence key that this device
//! did not make says the user is in a chat on the computer now, and the
//! phone shows the message without a sound.
//!
//! The runtime watches my own presence key in the subscription it holds
//! for my contacts' keys anyway (`presence`), and the handler of beats
//! writes down the newest beat of it that is not this device's own
//! ([`heard_mine`]). The push handler only reads that ([`elsewhere`]): no
//! question goes to a relay when a message comes, so no relay learns when
//! one came. A phone whose process is gone hears no beat, and its record
//! grows old: it rings as before.
//!
//! A beat of this device is written down before it goes out
//! ([`beating`]), so the copy that comes back from a relay, in this
//! process or the next, is never taken for another device's.

use crate::heartbeat::ONLINE_TTL_SECS;
use messenger_core::presence::{KEY_PRESENCE_ELSEWHERE, KEY_PRESENCE_LAST_BEAT, KEY_PRESENCE_MINE};
use messenger_core::Result;
use messenger_store::{settings, Store};

async fn get_i64(store: &Store, key: &str) -> Result<i64> {
    Ok(settings::get(store, key).await?.and_then(|s| s.parse().ok()).unwrap_or(0))
}

/// Whether the beat of another device of mine, seen at `seen_at` and good
/// until `until`, says it is in sight at `now`, when this device last beat
/// at `mine`. A beat older than one beat's promise counts not, whatever
/// its expiration says.
pub fn live(seen_at: i64, until: i64, now: i64, mine: i64) -> bool {
    seen_at > mine && now < until && now - seen_at < ONLINE_TTL_SECS
}

/// This device beats at `created_at`: written down before the beat goes.
pub async fn beating(store: &Store, created_at: i64) -> Result<()> {
    settings::set(store, KEY_PRESENCE_LAST_BEAT, &created_at.to_string()).await
}

/// My presence key now (hex), which the runtime watches with my contacts'.
pub async fn watching_mine(store: &Store, presence_pubkey: &str) -> Result<()> {
    if settings::get(store, KEY_PRESENCE_MINE).await?.as_deref() != Some(presence_pubkey) {
        settings::set(store, KEY_PRESENCE_MINE, presence_pubkey).await?;
    }
    Ok(())
}

/// Whether `presence_pubkey` is my presence key now.
pub async fn is_mine(store: &Store, presence_pubkey: &str) -> Result<bool> {
    Ok(settings::get(store, KEY_PRESENCE_MINE).await?.as_deref() == Some(presence_pubkey))
}

/// A beat of my presence key came: kept when another device made it and
/// it is newer than the one kept. `seen_at` and `until` as the handler
/// counts them (no further than my clock).
pub async fn heard_mine(store: &Store, seen_at: i64, until: i64) -> Result<bool> {
    if seen_at <= get_i64(store, KEY_PRESENCE_LAST_BEAT).await? {
        return Ok(false);
    }
    if let Some((kept, _)) = read(store).await? {
        if seen_at <= kept {
            return Ok(false);
        }
    }
    settings::set(store, KEY_PRESENCE_ELSEWHERE, &format!("{seen_at} {until}")).await?;
    Ok(true)
}

async fn read(store: &Store) -> Result<Option<(i64, i64)>> {
    Ok(settings::get(store, KEY_PRESENCE_ELSEWHERE).await?.and_then(|s| {
        let (seen_at, until) = s.split_once(' ')?;
        Some((seen_at.parse().ok()?, until.parse().ok()?))
    }))
}

/// Whether another device of mine is in sight at `now`, by what was heard
/// last. Reads the store only.
pub async fn elsewhere(store: &Store, now: i64) -> Result<bool> {
    let Some((seen_at, until)) = read(store).await? else { return Ok(false) };
    Ok(live(seen_at, until, now, get_i64(store, KEY_PRESENCE_LAST_BEAT).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_beat_of_another_device_counts_while_it_lasts() {
        assert!(live(1_000, 1_080, 1_050, 0));
        assert!(live(1_000, 1_080, 1_050, 990), "my own beat was earlier");
        assert!(!live(1_000, 1_080, 1_080, 0), "gone at its expiration");
        assert!(!live(1_000, 5_000, 1_080, 0), "older than a beat promises");
        assert!(!live(1_000, 1_080, 1_050, 1_000), "my own beat");
        assert!(!live(1_000, 1_080, 1_050, 1_010), "I beat since");
    }

    #[tokio::test]
    async fn my_own_beat_written_before_it_goes_is_not_another_device() {
        let store = Store::open_in_memory().await.unwrap();
        assert!(!elsewhere(&store, 1_050).await.unwrap(), "nothing heard");

        // This device beats at 1 000: its copy from the relay is passed over.
        beating(&store, 1_000).await.unwrap();
        assert!(!heard_mine(&store, 1_000, 1_080).await.unwrap());
        assert!(!elsewhere(&store, 1_010).await.unwrap());

        // The computer beats later.
        assert!(heard_mine(&store, 1_020, 1_100).await.unwrap());
        assert!(elsewhere(&store, 1_050).await.unwrap());
        assert!(!heard_mine(&store, 1_010, 1_090).await.unwrap(), "an older beat");
        assert!(!elsewhere(&store, 1_100).await.unwrap(), "expired");

        // This device beats after it: what was heard says nothing more.
        beating(&store, 1_030).await.unwrap();
        assert!(!elsewhere(&store, 1_050).await.unwrap());
    }

    #[tokio::test]
    async fn my_key_is_known_by_what_the_runtime_watches() {
        let store = Store::open_in_memory().await.unwrap();
        assert!(!is_mine(&store, "aa").await.unwrap());
        watching_mine(&store, "aa").await.unwrap();
        assert!(is_mine(&store, "aa").await.unwrap());
        watching_mine(&store, "bb").await.unwrap();
        assert!(!is_mine(&store, "aa").await.unwrap(), "the key moved on");
    }
}

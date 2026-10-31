// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The emoji I used last and use most, for the quick reactions and the
//! composer, on all my devices: the `RECENT` used last come first, the
//! newest of them first, then the most used of the others (ROO-297). Each device counts its own uses and sends the whole map to
//! the others now and then; a map that comes in raises each count and time
//! to the larger of the two. Only ever growing, so every device arrives at
//! the same map whatever order the snapshots come in.

use crate::{storage, Store};
use messenger_core::Result;

/// The most emoji a snapshot carries: the ones of `top`.
pub const SNAPSHOT_MAX: i64 = 64;

/// How many of the emoji used last stand before the most used: the first
/// line of the picker.
pub const RECENT: i64 = 7;

/// I used `emoji` at `at`: one more use, and the last time moves forward,
/// past the time of every other emoji: two used in the same second (or with
/// the clock behind) keep the order they were used in, here and on the
/// devices the snapshot reaches.
pub async fn bump(store: &Store, emoji: &str, at: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO msg_emoji_usage (emoji, count, last_at)
         VALUES (?1, 1, MAX(?2, COALESCE((SELECT MAX(last_at) + 1 FROM msg_emoji_usage WHERE emoji <> ?1), ?2)))
         ON CONFLICT(emoji) DO UPDATE SET count = msg_emoji_usage.count + 1, last_at = MAX(msg_emoji_usage.last_at, excluded.last_at)",
    )
    .bind(emoji)
    .bind(at)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// Take in a snapshot of `(emoji, count, last_at)` from another device of
/// mine: each count and time becomes the larger of the two. An entry with
/// no uses carries nothing and is skipped. `true` when anything changed.
pub async fn merge(store: &Store, usage: &[(String, i64, i64)]) -> Result<bool> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    let mut changed = false;
    for (emoji, count, last_at) in usage.iter().filter(|u| u.1 > 0) {
        let r = sqlx::query(
            "INSERT INTO msg_emoji_usage (emoji, count, last_at) VALUES (?, ?, ?)
             ON CONFLICT(emoji) DO UPDATE SET
               count = MAX(msg_emoji_usage.count, excluded.count), last_at = MAX(msg_emoji_usage.last_at, excluded.last_at)
             WHERE excluded.count > msg_emoji_usage.count OR excluded.last_at > msg_emoji_usage.last_at",
        )
        .bind(emoji)
        .bind(count)
        .bind(last_at)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        changed |= r.rows_affected() > 0;
    }
    tx.commit().await.map_err(storage)?;
    Ok(changed)
}

/// `n` emoji: the `RECENT` used last, the newest first, then the most used
/// of the others; of two used as often, the one used last first. Every
/// device with the same map gives the same order (a tie of times is broken
/// by the emoji).
pub async fn top(store: &Store, n: i64) -> Result<Vec<String>> {
    Ok(ordered(store, n).await?.into_iter().map(|(emoji, _, _)| emoji).collect())
}

/// The map to send to my other devices: `(emoji, count, last_at)`, the
/// `SNAPSHOT_MAX` of `top` in its order, so the ones used last go with
/// the most used and a cut takes the least used.
pub async fn all(store: &Store) -> Result<Vec<(String, i64, i64)>> {
    ordered(store, SNAPSHOT_MAX).await
}

async fn ordered(store: &Store, n: i64) -> Result<Vec<(String, i64, i64)>> {
    if n <= 0 {
        return Ok(Vec::new());
    }
    let mut out = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT emoji, count, last_at FROM msg_emoji_usage ORDER BY last_at DESC, emoji LIMIT ?",
    )
    .bind(RECENT.min(n))
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    let most = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT emoji, count, last_at FROM msg_emoji_usage ORDER BY count DESC, last_at DESC, emoji LIMIT ?",
    )
    .bind(n)
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    // The most used, as many as could be left after the recent ones.
    for row in most {
        if out.len() as i64 >= n {
            break;
        }
        if !out.iter().any(|(e, _, _)| *e == row.0) {
            out.push(row);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(emoji: &str, count: i64, at: i64) -> (String, i64, i64) {
        (emoji.into(), count, at)
    }

    #[tokio::test]
    async fn a_use_counts_and_the_time_only_moves_forward() {
        let s = Store::open_in_memory().await.unwrap();
        bump(&s, "👍", 100).await.unwrap();
        bump(&s, "👍", 300).await.unwrap();
        bump(&s, "👍", 200).await.unwrap();
        bump(&s, "❤️", 50).await.unwrap();
        // The clock behind: used after 👍, ❤️ still goes past it.
        assert_eq!(all(&s).await.unwrap(), vec![u("❤️", 1, 301), u("👍", 3, 300)]);
    }

    #[tokio::test]
    async fn two_used_in_the_same_second_keep_their_order() {
        let s = Store::open_in_memory().await.unwrap();
        bump(&s, "👍", 1000).await.unwrap();
        bump(&s, "😂", 1000).await.unwrap();
        assert_eq!(top(&s, 2).await.unwrap(), vec!["😂", "👍"]);
        bump(&s, "👍", 1000).await.unwrap();
        assert_eq!(top(&s, 2).await.unwrap(), vec!["👍", "😂"]);
        // Another device of mine takes the snapshot in: the same order there.
        let other = Store::open_in_memory().await.unwrap();
        assert!(merge(&other, &all(&s).await.unwrap()).await.unwrap());
        assert_eq!(top(&other, 2).await.unwrap(), vec!["👍", "😂"]);
    }

    #[tokio::test]
    async fn two_snapshots_agree_in_either_order() {
        let first = vec![u("👍", 5, 100), u("😂", 2, 400)];
        let second = vec![u("👍", 3, 200), u("🔥", 1, 50), u("😂", 4, 300)];
        // The used last first: there are fewer than `RECENT`.
        let want = vec![u("😂", 4, 400), u("👍", 5, 200), u("🔥", 1, 50)];

        let a = Store::open_in_memory().await.unwrap();
        assert!(merge(&a, &first).await.unwrap());
        assert!(merge(&a, &second).await.unwrap());
        let b = Store::open_in_memory().await.unwrap();
        assert!(merge(&b, &second).await.unwrap());
        assert!(merge(&b, &first).await.unwrap());
        assert_eq!(all(&a).await.unwrap(), want);
        assert_eq!(all(&b).await.unwrap(), want);

        assert!(!merge(&a, &first).await.unwrap(), "the same snapshot again changes nothing");
        assert!(!merge(&a, &[u("👍", 1, 10)]).await.unwrap(), "an older one neither");
        assert!(!merge(&a, &[u("🙂", 0, 999)]).await.unwrap(), "no uses, nothing to keep");
        assert!(!merge(&a, &[]).await.unwrap());
        assert!(merge(&a, &[u("👍", 5, 201)]).await.unwrap(), "a later time alone is news");
    }

    #[tokio::test]
    async fn a_use_here_and_a_snapshot_from_elsewhere_meet() {
        let s = Store::open_in_memory().await.unwrap();
        bump(&s, "👍", 100).await.unwrap();
        assert!(merge(&s, &[u("👍", 4, 90)]).await.unwrap(), "the other device used it more");
        bump(&s, "👍", 110).await.unwrap();
        assert_eq!(all(&s).await.unwrap(), vec![u("👍", 5, 110)]);
        assert!(!merge(&s, &[u("👍", 5, 110)]).await.unwrap(), "my own snapshot coming back");
    }

    #[tokio::test]
    async fn the_used_last_come_first_then_the_most_used() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(top(&s, 6).await.unwrap().is_empty());
        // Ten used often long ago, and one used once just now.
        let often: Vec<_> = (0..10).map(|i| u(&format!("f{i}"), 100 - i, 10 + i)).collect();
        merge(&s, &often).await.unwrap();
        bump(&s, "🦔", 1000).await.unwrap();
        let got = top(&s, 24).await.unwrap();
        assert_eq!(got.len(), 11);
        // The first line: the newest first, the hedgehog before all.
        assert_eq!(&got[..RECENT as usize], ["🦔", "f9", "f8", "f7", "f6", "f5", "f4"]);
        // Then the most used of the others, no one twice.
        assert_eq!(&got[RECENT as usize..], ["f0", "f1", "f2", "f3"]);
        assert_eq!(top(&s, 2).await.unwrap(), vec!["🦔", "f9"]);
        // Used again, an old one is the newest.
        bump(&s, "f0", 1001).await.unwrap();
        assert_eq!(top(&s, 3).await.unwrap(), vec!["f0", "🦔", "f9"]);
    }

    #[tokio::test]
    async fn ties_are_broken_the_same_on_every_device() {
        let s = Store::open_in_memory().await.unwrap();
        merge(&s, &[u("😂", 3, 100), u("👍", 3, 300), u("🅰️", 3, 300)]).await.unwrap();
        // Time equal: by the bytes of the emoji, U+1F170 before U+1F44D.
        assert_eq!(top(&s, 3).await.unwrap(), vec!["🅰️", "👍", "😂"]);
        // Past the first line: count equal, the one used last first.
        let many: Vec<_> = (0..RECENT).map(|i| u(&format!("n{i}"), 1, 500 + i)).collect();
        merge(&s, &many).await.unwrap();
        assert_eq!(&top(&s, 24).await.unwrap()[RECENT as usize..], ["🅰️", "👍", "😂"]);
    }

    #[tokio::test]
    async fn a_snapshot_carries_the_most_used_only() {
        let s = Store::open_in_memory().await.unwrap();
        let many: Vec<_> = (0..(SNAPSHOT_MAX + 6)).map(|i| u(&format!("e{i:03}"), i + 1, i)).collect();
        merge(&s, &many).await.unwrap();
        let snap = all(&s).await.unwrap();
        assert_eq!(snap.len(), SNAPSHOT_MAX as usize);
        assert_eq!(snap[0].1, SNAPSHOT_MAX + 6, "the most used first");
        assert_eq!(snap.last().unwrap().1, 7, "the six least used are left out");

        // One used once just now goes too, at the head: the other devices put it first as well.
        bump(&s, "🦔", 10_000).await.unwrap();
        let snap = all(&s).await.unwrap();
        assert_eq!(snap.len(), SNAPSHOT_MAX as usize);
        assert_eq!(snap[0], u("🦔", 1, 10_000));
        assert_eq!(snap.last().unwrap().1, 8, "a less used one gave way");
    }
}

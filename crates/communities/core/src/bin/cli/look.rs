// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a bot holds of a channel, read from its database: the counts the
//! trial compares.

use crate::bot::Bot;
use communities_core::db;
use communities_core::state::SCOPE_EVERYONE;
use messenger_core::{MessengerError, Result};
use serde::Serialize;

fn storage(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Storage(e.to_string())
}

/// Posts whose text starts with `marker`.
pub async fn posts(bot: &Bot, cid: &str, marker: &str) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM cm_posts WHERE community_id = ? AND text LIKE ? AND deleted = 0")
        .bind(cid)
        .bind(format!("{marker}%"))
        .fetch_one(bot.svc.db().pool())
        .await
        .map_err(storage)
}

/// Comments whose text starts with `marker`.
pub async fn comments(bot: &Bot, cid: &str, marker: &str) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM cm_comments WHERE community_id = ? AND text LIKE ? AND deleted = 0")
        .bind(cid)
        .bind(format!("{marker}%"))
        .fetch_one(bot.svc.db().pool())
        .await
        .map_err(storage)
}

/// Every post (any text), and those with no text yet (sealed under a key
/// not held).
pub async fn post_ids(bot: &Bot, cid: &str, marker: &str) -> Result<Vec<String>> {
    sqlx::query_scalar("SELECT id FROM cm_posts WHERE community_id = ? AND text LIKE ? ORDER BY created_at, id")
        .bind(cid)
        .bind(format!("{marker}%"))
        .fetch_all(bot.svc.db().pool())
        .await
        .map_err(storage)
}

/// The epochs whose key is held.
pub async fn held(bot: &Bot, cid: &str) -> Result<Vec<i64>> {
    Ok(db::keys::epochs(bot.svc.db(), cid, SCOPE_EVERYONE).await?.into_iter().filter(|r| r.key_hex.is_some()).map(|r| r.epoch).collect())
}

/// The epochs whose event is here.
pub async fn known_epochs(bot: &Bot, cid: &str) -> Result<Vec<i64>> {
    Ok(db::keys::epochs(bot.svc.db(), cid, SCOPE_EVERYONE).await?.into_iter().map(|r| r.epoch).collect())
}

pub async fn membership(bot: &Bot, cid: &str) -> Result<String> {
    Ok(db::channels::get(bot.svc.db(), cid).await?.map(|r| r.membership).unwrap_or_else(|| "unknown".into()))
}

/// The shards of the newest snapshot here that are cached.
pub async fn shards(bot: &Bot, cid: &str) -> Result<(usize, usize)> {
    let Some(snap) = bot.svc.member(&bot.keys).latest_snapshot(cid).await? else { return Ok((0, 0)) };
    let mut cached = 0;
    for s in &snap.shards {
        if db::snapshots::shard(bot.svc.db(), cid, &s.from, &s.sha256).await?.is_some() {
            cached += 1;
        }
    }
    Ok((cached, snap.shards.len()))
}

/// What one bot holds, for a check.
#[derive(Clone, Debug, Serialize)]
pub struct Look {
    pub bot: String,
    pub membership: String,
    pub held: Vec<i64>,
    pub posts: i64,
    pub comments: i64,
}

pub async fn look(bot: &Bot, cid: &str, post_marker: &str, comment_marker: &str) -> Result<Look> {
    Ok(Look {
        bot: bot.name.clone(),
        membership: membership(bot, cid).await?,
        held: held(bot, cid).await?,
        posts: posts(bot, cid, post_marker).await?,
        comments: comments(bot, cid, comment_marker).await?,
    })
}

/// Every post and comment held, whatever its text.
pub async fn rows(bot: &Bot, cid: &str) -> Result<i64> {
    let p: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cm_posts WHERE community_id = ?").bind(cid).fetch_one(bot.svc.db().pool()).await.map_err(storage)?;
    let c: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cm_comments WHERE community_id = ?").bind(cid).fetch_one(bot.svc.db().pool()).await.map_err(storage)?;
    Ok(p + c)
}

/// Comments of `marker` (`<marker> <bot> <n>`) whose bot is none of
/// `except` (readers banned later: a device that comes after the ban
/// refuses what they wrote before it, one that was there keeps it).
pub async fn comments_except(bot: &Bot, cid: &str, marker: &str, except: &[String]) -> Result<i64> {
    if except.is_empty() {
        return comments(bot, cid, marker).await;
    }
    let marks = vec!["?"; except.len()].join(", ");
    let sql = format!("SELECT COUNT(*) FROM cm_comments WHERE community_id = ? AND text LIKE ? AND deleted = 0 AND substr(text, ?, ?) NOT IN ({marks})");
    let width = except.iter().map(String::len).max().unwrap_or(0);
    let mut q = sqlx::query_scalar(sqlx::AssertSqlSafe(sql)).bind(cid).bind(format!("{marker}%")).bind(marker.len() as i64 + 2).bind(width as i64);
    for e in except {
        q = q.bind(e);
    }
    q.fetch_one(bot.svc.db().pool()).await.map_err(storage)
}

/// The epochs of a scope whose key is held.
pub async fn held_in(bot: &Bot, cid: &str, scope_id: &str) -> Result<Vec<i64>> {
    Ok(db::keys::epochs(bot.svc.db(), cid, scope_id).await?.into_iter().filter(|r| r.key_hex.is_some()).map(|r| r.epoch).collect())
}

/// The posts of a scope held: every row, and those opened whose text
/// starts with `marker`.
pub async fn posts_in(bot: &Bot, cid: &str, scope_id: &str, marker: &str) -> Result<(i64, i64)> {
    let all: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cm_posts WHERE community_id = ? AND scope_id = ? AND deleted = 0")
        .bind(cid)
        .bind(scope_id)
        .fetch_one(bot.svc.db().pool())
        .await
        .map_err(storage)?;
    let open: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cm_posts WHERE community_id = ? AND scope_id = ? AND text LIKE ? AND deleted = 0")
        .bind(cid)
        .bind(scope_id)
        .bind(format!("{marker}%"))
        .fetch_one(bot.svc.db().pool())
        .await
        .map_err(storage)?;
    Ok((all, open))
}

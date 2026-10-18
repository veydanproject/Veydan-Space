// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Group tables. Meaning lives in `messenger-groups`; this is storage.

use crate::{storage, Store};
use messenger_core::Result;

pub const KIND_GROUP: &str = "group";

pub fn group_chat_id(group_id: &str) -> String {
    format!("group:{group_id}")
}

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct GroupRow {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub about: String,
    pub picture: String,
    pub relay_url: String,
    pub owner: String,
    pub membership: String,
    pub my_role: Option<String>,
    pub members: i64,
    pub link_epoch: i64,
    pub created_at: i64,
    pub updated_at: i64,
    /// Something of the group is newer than this build reads
    /// (`set_needs_update`); not touched by `upsert`.
    pub needs_update: bool,
}

const COLS: &str = "id, kind, name, about, picture, relay_url, owner, membership, my_role, members, link_epoch, created_at, updated_at, needs_update";

/// Insert or refresh the cached facts of a group. The relay is kept in its
/// normal form (`servers::normalize`), the one a server is found by.
pub async fn upsert(store: &Store, g: &GroupRow) -> Result<()> {
    let now = crate::now();
    sqlx::query(
        "INSERT INTO msg_groups (id, kind, name, about, picture, relay_url, owner, membership, my_role, members, link_epoch, created_at, updated_at, needs_update)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, about = excluded.about, picture = excluded.picture,
           relay_url = excluded.relay_url, owner = excluded.owner, membership = excluded.membership,
           my_role = excluded.my_role, members = excluded.members, link_epoch = excluded.link_epoch,
           updated_at = excluded.updated_at",
    )
    .bind(&g.id)
    .bind(&g.kind)
    .bind(&g.name)
    .bind(&g.about)
    .bind(&g.picture)
    .bind(crate::servers::normalize(&g.relay_url))
    .bind(&g.owner)
    .bind(&g.membership)
    .bind(&g.my_role)
    .bind(g.members)
    .bind(g.link_epoch)
    .bind(now)
    .bind(now)
    .bind(g.needs_update)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    // The chat row of the group, with what my devices chose of it.
    let chat = group_chat_id(&g.id);
    let made = sqlx::query(
        "INSERT OR IGNORE INTO msg_chats (id, kind, peer_pubkey, unread, last_message_at, last_preview, pinned, archived, muted, created_at, updated_at)
         VALUES (?, 'group', NULL, 0, NULL, NULL, 0, 0, 0, ?, ?)",
    )
    .bind(&chat)
    .bind(now)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    if made.rows_affected() > 0 {
        crate::chats::take_prefs(store, &chat).await?;
    }
    Ok(())
}

pub async fn get(store: &Store, id: &str) -> Result<Option<GroupRow>> {
    sqlx::query_as::<_, GroupRow>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM msg_groups WHERE id = ?")))
        .bind(id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

pub async fn list(store: &Store) -> Result<Vec<GroupRow>> {
    sqlx::query_as::<_, GroupRow>(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM msg_groups ORDER BY updated_at DESC")))
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

/// Whether something of the group waits for a newer build of the app.
pub async fn set_needs_update(store: &Store, id: &str, needs_update: bool) -> Result<()> {
    sqlx::query("UPDATE msg_groups SET needs_update = ? WHERE id = ?")
        .bind(needs_update)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Forget a group on this device: cache, log, keys list, pending, chat.
pub async fn delete(store: &Store, id: &str) -> Result<()> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    for sql in [
        "DELETE FROM msg_group_ops WHERE group_id = ?",
        "DELETE FROM msg_group_keys WHERE group_id = ?",
        "DELETE FROM msg_group_pending WHERE group_id = ?",
        "DELETE FROM msg_group_invites WHERE group_id = ?",
        "DELETE FROM msg_group_requests WHERE group_id = ?",
        "DELETE FROM msg_groups WHERE id = ?",
    ] {
        sqlx::query(sql).bind(id).execute(&mut *tx).await.map_err(storage)?;
    }
    let chat = group_chat_id(id);
    sqlx::query("DELETE FROM msg_messages WHERE chat_id = ?").bind(&chat).execute(&mut *tx).await.map_err(storage)?;
    sqlx::query("DELETE FROM msg_chats WHERE id = ?").bind(&chat).execute(&mut *tx).await.map_err(storage)?;
    tx.commit().await.map_err(storage)
}

// ─── Keys owed to the secret store ──────────────────────────────────────────

/// The keys of groups forgotten with their server (`servers::remove_private`)
/// whose secrets the group service has yet to delete: `[[group, key], …]`
/// in `msg_settings`. The removal comes by the snapshot too, in a crate
/// that holds no secret store; the group service empties the list
/// (`GroupService::forget_gone`).
pub const SECRETS_OWED: &str = "groups.secrets_owed";

pub(crate) async fn secrets_owed_in(db: &mut sqlx::SqliteConnection) -> Result<Vec<(String, String)>> {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM msg_settings WHERE key = ?")
        .bind(SECRETS_OWED)
        .fetch_optional(&mut *db)
        .await
        .map_err(storage)?;
    Ok(value.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
}

pub(crate) async fn set_secrets_owed_in(db: &mut sqlx::SqliteConnection, owed: &[(String, String)]) -> Result<()> {
    if owed.is_empty() {
        sqlx::query("DELETE FROM msg_settings WHERE key = ?").bind(SECRETS_OWED).execute(&mut *db).await.map_err(storage)?;
        return Ok(());
    }
    let value = serde_json::to_string(owed)?;
    sqlx::query("INSERT INTO msg_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(SECRETS_OWED)
        .bind(value)
        .execute(&mut *db)
        .await
        .map_err(storage)?;
    Ok(())
}

/// The keys whose secrets are owed, `(group, key)`.
pub async fn secrets_owed(store: &Store) -> Result<Vec<(String, String)>> {
    let mut conn = store.pool().acquire().await.map_err(storage)?;
    secrets_owed_in(&mut conn).await
}

/// Those of `done` are owed no more.
pub async fn secrets_forgotten(store: &Store, done: &[(String, String)]) -> Result<()> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    let mut owed = secrets_owed_in(&mut tx).await?;
    owed.retain(|k| !done.contains(k));
    set_secrets_owed_in(&mut tx, &owed).await?;
    tx.commit().await.map_err(storage)
}

// ─── Operations ─────────────────────────────────────────────────────────────

/// `false` when the operation was already stored.
pub async fn insert_op(store: &Store, group_id: &str, op_id: &str, op_json: &str) -> Result<bool> {
    let res = sqlx::query("INSERT OR IGNORE INTO msg_group_ops (op_id, group_id, op_json, received_at) VALUES (?, ?, ?, ?)")
        .bind(op_id)
        .bind(group_id)
        .bind(op_json)
        .bind(crate::now())
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn ops(store: &Store, group_id: &str) -> Result<Vec<String>> {
    sqlx::query_scalar::<_, String>("SELECT op_json FROM msg_group_ops WHERE group_id = ? ORDER BY received_at, op_id")
        .bind(group_id)
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

// ─── Keys (ids only) ────────────────────────────────────────────────────────

pub async fn add_key(store: &Store, group_id: &str, key_id: &str) -> Result<bool> {
    let res = sqlx::query("INSERT OR IGNORE INTO msg_group_keys (group_id, key_id, created_at) VALUES (?, ?, ?)")
        .bind(group_id)
        .bind(key_id)
        .bind(crate::now())
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn remove_key(store: &Store, group_id: &str, key_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_group_keys WHERE group_id = ? AND key_id = ?")
        .bind(group_id)
        .bind(key_id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn key_ids(store: &Store, group_id: &str) -> Result<Vec<String>> {
    sqlx::query_scalar::<_, String>("SELECT key_id FROM msg_group_keys WHERE group_id = ? ORDER BY created_at, key_id")
        .bind(group_id)
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

// ─── Pending ciphertext ─────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct PendingRow {
    pub event_id: String,
    pub group_id: String,
    pub key_id: String,
    pub ciphertext: String,
    pub created_at: i64,
    pub received_at: i64,
}

pub async fn add_pending(store: &Store, p: &PendingRow) -> Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO msg_group_pending (event_id, group_id, key_id, ciphertext, created_at, received_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&p.event_id)
    .bind(&p.group_id)
    .bind(&p.key_id)
    .bind(&p.ciphertext)
    .bind(p.created_at)
    .bind(crate::now())
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

/// Take (and remove) everything that waits for this key, oldest first.
pub async fn take_pending(store: &Store, group_id: &str, key_id: &str) -> Result<Vec<PendingRow>> {
    let rows = sqlx::query_as::<_, PendingRow>(
        "SELECT event_id, group_id, key_id, ciphertext, created_at, received_at FROM msg_group_pending
         WHERE group_id = ? AND key_id = ? ORDER BY created_at, event_id",
    )
    .bind(group_id)
    .bind(key_id)
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    sqlx::query("DELETE FROM msg_group_pending WHERE group_id = ? AND key_id = ?")
        .bind(group_id)
        .bind(key_id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(rows)
}

/// The groups that have something held under `key_id`, in id order.
pub async fn pending_groups(store: &Store, key_id: &str) -> Result<Vec<String>> {
    sqlx::query_scalar::<_, String>("SELECT DISTINCT group_id FROM msg_group_pending WHERE key_id = ? ORDER BY group_id")
        .bind(key_id)
        .fetch_all(store.pool())
        .await
        .map_err(storage)
}

/// What waits in a bucket, oldest first, left where it is.
pub async fn pending_in(store: &Store, group_id: &str, key_id: &str) -> Result<Vec<PendingRow>> {
    sqlx::query_as::<_, PendingRow>(
        "SELECT event_id, group_id, key_id, ciphertext, created_at, received_at FROM msg_group_pending
         WHERE group_id = ? AND key_id = ? ORDER BY created_at, event_id",
    )
    .bind(group_id)
    .bind(key_id)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

pub async fn delete_pending(store: &Store, event_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_group_pending WHERE event_id = ?").bind(event_id).execute(store.pool()).await.map_err(storage)?;
    Ok(())
}

/// How much waits in the buckets of a group whose name starts with
/// `prefix`: rows, and bytes of ciphertext.
pub async fn pending_prefix_size(store: &Store, group_id: &str, prefix: &str) -> Result<(i64, i64)> {
    sqlx::query_as::<_, (i64, i64)>(
        "SELECT COUNT(*), COALESCE(SUM(length(ciphertext)), 0) FROM msg_group_pending
         WHERE group_id = ? AND substr(key_id, 1, length(?)) = ?",
    )
    .bind(group_id)
    .bind(prefix)
    .bind(prefix)
    .fetch_one(store.pool())
    .await
    .map_err(storage)
}

/// Drop the row of those buckets that came first (by its receipt here,
/// never by the date its author wrote on it: a bucket of what strangers
/// to this build wrote is emptied as a queue, and a date is anybody's
/// word); the bytes it held, or none when there was nothing to drop.
pub async fn delete_oldest_pending(store: &Store, group_id: &str, prefix: &str) -> Result<Option<i64>> {
    let oldest = sqlx::query_as::<_, (String, i64)>(
        "SELECT event_id, length(ciphertext) FROM msg_group_pending
         WHERE group_id = ? AND substr(key_id, 1, length(?)) = ? ORDER BY received_at, rowid LIMIT 1",
    )
    .bind(group_id)
    .bind(prefix)
    .bind(prefix)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)?;
    let Some((event_id, bytes)) = oldest else { return Ok(None) };
    delete_pending(store, &event_id).await?;
    Ok(Some(bytes))
}

/// Events that wait for a key (not those that wait for their author),
/// dated `since` or later.
pub async fn count_pending(store: &Store, group_id: &str, since: i64) -> Result<i64> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM msg_group_pending WHERE group_id = ? AND length(key_id) = 32 AND created_at >= ?")
        .bind(group_id)
        .bind(since)
        .fetch_one(store.pool())
        .await
        .map_err(storage)
}

// ─── Invites ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct InviteRow {
    pub invite_id: String,
    pub group_id: String,
    pub direction: String,
    pub peer: String,
    pub status: String,
    pub payload_json: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub updated_at: i64,
}

const I_COLS: &str = "invite_id, group_id, direction, peer, status, payload_json, created_at, expires_at, updated_at";

/// `false` when an invite with this id exists already.
pub async fn insert_invite(store: &Store, i: &InviteRow) -> Result<bool> {
    let res = sqlx::query(
        "INSERT OR IGNORE INTO msg_group_invites (invite_id, group_id, direction, peer, status, payload_json, created_at, expires_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&i.invite_id)
    .bind(&i.group_id)
    .bind(&i.direction)
    .bind(&i.peer)
    .bind(&i.status)
    .bind(&i.payload_json)
    .bind(i.created_at)
    .bind(i.expires_at)
    .bind(crate::now())
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected() == 1)
}

pub async fn invite(store: &Store, invite_id: &str) -> Result<Option<InviteRow>> {
    sqlx::query_as::<_, InviteRow>(sqlx::AssertSqlSafe(format!("SELECT {I_COLS} FROM msg_group_invites WHERE invite_id = ?")))
        .bind(invite_id)
        .fetch_optional(store.pool())
        .await
        .map_err(storage)
}

pub async fn invites(store: &Store, direction: &str, statuses: &[&str]) -> Result<Vec<InviteRow>> {
    let all = sqlx::query_as::<_, InviteRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {I_COLS} FROM msg_group_invites WHERE direction = ? ORDER BY created_at DESC"
    )))
    .bind(direction)
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    Ok(all.into_iter().filter(|i| statuses.is_empty() || statuses.contains(&i.status.as_str())).collect())
}

pub async fn set_invite_status(store: &Store, invite_id: &str, status: &str) -> Result<()> {
    sqlx::query("UPDATE msg_group_invites SET status = ?, updated_at = ? WHERE invite_id = ?")
        .bind(status)
        .bind(crate::now())
        .bind(invite_id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// Open invites past their time become `expired`.
pub async fn set_invite_payload(store: &Store, invite_id: &str, payload_json: &str) -> Result<()> {
    sqlx::query("UPDATE msg_group_invites SET payload_json = ? WHERE invite_id = ?")
        .bind(payload_json)
        .bind(invite_id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

pub async fn expire_invites(store: &Store, now: i64) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE msg_group_invites SET status = 'expired', updated_at = ?
         WHERE status IN ('sent', 'received') AND expires_at <= ?",
    )
    .bind(now)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected())
}

/// Incoming invitations past their date that were never answered: gone.
pub async fn delete_lapsed_invites(store: &Store, now: i64) -> Result<u64> {
    let res = sqlx::query(
        "DELETE FROM msg_group_invites WHERE direction = 'in' AND status IN ('received', 'expired') AND expires_at <= ?",
    )
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(res.rows_affected())
}

pub async fn delete_invite(store: &Store, invite_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_group_invites WHERE invite_id = ?")
        .bind(invite_id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

// ─── Join requests ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct RequestRow {
    pub group_id: String,
    pub requester: String,
    pub direction: String,
    pub status: String,
    pub payload_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

pub async fn put_request(store: &Store, r: &RequestRow) -> Result<()> {
    let now = crate::now();
    sqlx::query(
        "INSERT INTO msg_group_requests (group_id, requester, direction, status, payload_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(group_id, requester, direction) DO UPDATE SET status = excluded.status,
           payload_json = excluded.payload_json, created_at = excluded.created_at, updated_at = excluded.updated_at",
    )
    .bind(&r.group_id)
    .bind(&r.requester)
    .bind(&r.direction)
    .bind(&r.status)
    .bind(&r.payload_json)
    .bind(r.created_at)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(())
}

pub async fn request(store: &Store, group_id: &str, requester: &str, direction: &str) -> Result<Option<RequestRow>> {
    sqlx::query_as::<_, RequestRow>(
        "SELECT group_id, requester, direction, status, payload_json, created_at, updated_at
         FROM msg_group_requests WHERE group_id = ? AND requester = ? AND direction = ?",
    )
    .bind(group_id)
    .bind(requester)
    .bind(direction)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

pub async fn requests(store: &Store, group_id: &str, direction: &str, status: &str) -> Result<Vec<RequestRow>> {
    sqlx::query_as::<_, RequestRow>(
        "SELECT group_id, requester, direction, status, payload_json, created_at, updated_at
         FROM msg_group_requests WHERE group_id = ? AND direction = ? AND status = ? ORDER BY created_at",
    )
    .bind(group_id)
    .bind(direction)
    .bind(status)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

pub async fn outgoing_requests(store: &Store, status: &str) -> Result<Vec<RequestRow>> {
    sqlx::query_as::<_, RequestRow>(
        "SELECT group_id, requester, direction, status, payload_json, created_at, updated_at
         FROM msg_group_requests WHERE direction = 'out' AND status = ? ORDER BY created_at DESC",
    )
    .bind(status)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(id: &str) -> GroupRow {
        GroupRow {
            id: id.into(),
            kind: "private".into(),
            name: "Team".into(),
            about: String::new(),
            picture: String::new(),
            relay_url: "wss://r.example".into(),
            owner: "o".into(),
            membership: "joined".into(),
            my_role: Some("owner".into()),
            members: 1,
            link_epoch: 0,
            created_at: 0,
            updated_at: 0,
            needs_update: false,
        }
    }

    #[tokio::test]
    async fn groups_ops_keys_pending() {
        let s = Store::open_in_memory().await.unwrap();
        upsert(&s, &group("g1")).await.unwrap();
        let mut g = group("g1");
        g.name = "Renamed".into();
        g.members = 3;
        upsert(&s, &g).await.unwrap();
        assert_eq!(get(&s, "g1").await.unwrap().unwrap().name, "Renamed");
        assert_eq!(list(&s).await.unwrap().len(), 1);
        assert!(crate::chats::get(&s, "group:g1").await.unwrap().is_some(), "the chat row comes with the group");

        assert!(insert_op(&s, "g1", "op1", "{}").await.unwrap());
        assert!(!insert_op(&s, "g1", "op1", "{}").await.unwrap());
        insert_op(&s, "g1", "op2", "{\"a\":1}").await.unwrap();
        assert_eq!(ops(&s, "g1").await.unwrap().len(), 2);

        assert!(add_key(&s, "g1", "k1").await.unwrap());
        assert!(!add_key(&s, "g1", "k1").await.unwrap());
        assert_eq!(key_ids(&s, "g1").await.unwrap(), vec!["k1".to_string()]);

        for (id, at) in [("e2", 20), ("e1", 10)] {
            add_pending(&s, &PendingRow { event_id: id.into(), group_id: "g1".into(), key_id: "k9k9k9k9k9k9k9k9k9k9k9k9k9k9k9k9".into(), ciphertext: "c".into(), created_at: at, received_at: 0 })
                .await
                .unwrap();
        }
        assert_eq!(count_pending(&s, "g1", 0).await.unwrap(), 2);
        let taken = take_pending(&s, "g1", "k9k9k9k9k9k9k9k9k9k9k9k9k9k9k9k9").await.unwrap();
        assert_eq!(taken.iter().map(|p| p.event_id.as_str()).collect::<Vec<_>>(), vec!["e1", "e2"]);
        assert_eq!(count_pending(&s, "g1", 0).await.unwrap(), 0);

        // Buckets of one prefix: measured, read in place, the first that
        // came dropped (f2 came first, whatever its date says).
        for (id, bucket, at, text) in [("f2", "future:k1", 20, "cc"), ("f1", "future:k2", 10, "c"), ("w1", "welcome", 5, "cccc")] {
            add_pending(&s, &PendingRow { event_id: id.into(), group_id: "g1".into(), key_id: bucket.into(), ciphertext: text.into(), created_at: at, received_at: 0 })
                .await
                .unwrap();
        }
        assert_eq!(pending_prefix_size(&s, "g1", "future:").await.unwrap(), (2, 3));
        assert_eq!(pending_prefix_size(&s, "g1", "nothing:").await.unwrap(), (0, 0));
        assert_eq!(count_pending(&s, "g1", 0).await.unwrap(), 0, "none of them waits for a key");
        assert_eq!(pending_in(&s, "g1", "future:k1").await.unwrap().iter().map(|p| p.event_id.as_str()).collect::<Vec<_>>(), vec!["f2"]);
        assert_eq!(pending_in(&s, "g1", "future:k1").await.unwrap().len(), 1, "reading leaves it there");
        assert_eq!(delete_oldest_pending(&s, "g1", "future:").await.unwrap(), Some(2));
        assert_eq!(pending_prefix_size(&s, "g1", "future:").await.unwrap(), (1, 1));
        assert!(pending_in(&s, "g1", "future:k1").await.unwrap().is_empty(), "f2 went, though dated later");
        delete_pending(&s, "f1").await.unwrap();
        assert_eq!(delete_oldest_pending(&s, "g1", "future:").await.unwrap(), None);
        assert_eq!(pending_in(&s, "g1", "welcome").await.unwrap().len(), 1, "another bucket is untouched");

        assert!(!get(&s, "g1").await.unwrap().unwrap().needs_update);
        set_needs_update(&s, "g1", true).await.unwrap();
        assert!(get(&s, "g1").await.unwrap().unwrap().needs_update);
        upsert(&s, &group("g1")).await.unwrap();
        assert!(get(&s, "g1").await.unwrap().unwrap().needs_update, "a refresh of the row does not clear it");
        set_needs_update(&s, "g1", false).await.unwrap();
        assert!(!get(&s, "g1").await.unwrap().unwrap().needs_update);

        delete(&s, "g1").await.unwrap();
        assert!(get(&s, "g1").await.unwrap().is_none());
        assert!(ops(&s, "g1").await.unwrap().is_empty());
        assert!(crate::chats::get(&s, "group:g1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn invites_and_requests() {
        let s = Store::open_in_memory().await.unwrap();
        let inv = InviteRow {
            invite_id: "i1".into(),
            group_id: "g1".into(),
            direction: "in".into(),
            peer: "p".into(),
            status: "received".into(),
            payload_json: "{}".into(),
            created_at: 100,
            expires_at: 200,
            updated_at: 0,
        };
        assert!(insert_invite(&s, &inv).await.unwrap());
        assert!(!insert_invite(&s, &inv).await.unwrap(), "same id twice");
        assert_eq!(invites(&s, "in", &["received"]).await.unwrap().len(), 1);
        assert!(invites(&s, "out", &[]).await.unwrap().is_empty());
        assert_eq!(expire_invites(&s, 150).await.unwrap(), 0);
        assert_eq!(expire_invites(&s, 200).await.unwrap(), 1);
        assert_eq!(invite(&s, "i1").await.unwrap().unwrap().status, "expired");
        set_invite_status(&s, "i1", "declined").await.unwrap();
        assert_eq!(expire_invites(&s, 999).await.unwrap(), 0, "only open invites expire");

        let req = RequestRow { group_id: "g1".into(), requester: "x".into(), direction: "in".into(), status: "pending".into(), payload_json: "{}".into(), created_at: 1, updated_at: 0 };
        put_request(&s, &req).await.unwrap();
        put_request(&s, &RequestRow { direction: "out".into(), ..req.clone() }).await.unwrap();
        assert_eq!(requests(&s, "g1", "in", "pending").await.unwrap().len(), 1);
        assert_eq!(outgoing_requests(&s, "pending").await.unwrap().len(), 1);
        put_request(&s, &RequestRow { status: "approved".into(), ..req }).await.unwrap();
        assert!(requests(&s, "g1", "in", "pending").await.unwrap().is_empty());
        assert_eq!(request(&s, "g1", "x", "in").await.unwrap().unwrap().status, "approved");
    }
}

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
}

const COLS: &str = "id, kind, name, about, picture, relay_url, owner, membership, my_role, members, link_epoch, created_at, updated_at";

/// Insert or refresh the cached facts of a group.
pub async fn upsert(store: &Store, g: &GroupRow) -> Result<()> {
    let now = crate::now();
    sqlx::query(
        "INSERT INTO msg_groups (id, kind, name, about, picture, relay_url, owner, membership, my_role, members, link_epoch, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
    .bind(&g.relay_url)
    .bind(&g.owner)
    .bind(&g.membership)
    .bind(&g.my_role)
    .bind(g.members)
    .bind(g.link_epoch)
    .bind(now)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    // The chat row of the group.
    sqlx::query(
        "INSERT OR IGNORE INTO msg_chats (id, kind, peer_pubkey, unread, last_message_at, last_preview, pinned, archived, muted, created_at, updated_at)
         VALUES (?, 'group', NULL, 0, NULL, NULL, 0, 0, 0, ?, ?)",
    )
    .bind(group_chat_id(&g.id))
    .bind(now)
    .bind(now)
    .execute(store.pool())
    .await
    .map_err(storage)?;
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
           payload_json = excluded.payload_json, updated_at = excluded.updated_at",
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

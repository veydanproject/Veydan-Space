// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `msg_servers`: the servers the messenger knows (migration 039). A
//! server is one relay with its app key, and what goes with it (a blob
//! server, a call node, a push mode, a name). Its id is derived from the
//! relay url (`id_of`), so a server is named the same on every device and
//! a row can never name another server's relay.
//!
//! `home` rows mirror the relays used as before (one per relay url, kept by
//! the runtime's `RelayService::reload`); `private` rows come from a server
//! card, and nothing is sent to them or heard from them unless a scope
//! names them.

use crate::{storage, Store};
use messenger_core::{MessengerError, Result};
use sha2::{Digest, Sha256};

pub const KIND_HOME: &str = "home";
pub const KIND_PRIVATE: &str = "private";

pub const PUSH_OFF: &str = "off";
pub const PUSH_VEYDAN: &str = "veydan";

/// How many hex characters of the SHA-256 make a server id: 64 bits, far
/// more than the servers one person will ever add.
const ID_LEN: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct ServerRow {
    pub id: String,
    /// `home` | `private`
    pub kind: String,
    pub name: Option<String>,
    pub relay_url: String,
    /// The app key the relay (and its blob server) wants as `?key=`.
    /// Never shown to the UI, never logged.
    pub relay_key: Option<String>,
    pub media_url: Option<String>,
    pub call_node: Option<String>,
    /// `off` | `veydan`
    pub push_mode: String,
    pub source: Option<String>,
    pub added_at: i64,
}

impl ServerRow {
    pub fn is_private(&self) -> bool {
        self.kind == KIND_PRIVATE
    }
}

/// The form a relay url is compared and hashed in: trimmed, lowercase,
/// without a trailing slash.
pub fn normalize(relay_url: &str) -> String {
    relay_url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// The id of the server of `relay_url`: the first 16 hex of the SHA-256 of
/// its normal form. `wss://Relay.example/` and `wss://relay.example` are
/// one server.
pub fn id_of(relay_url: &str) -> String {
    let digest = Sha256::digest(normalize(relay_url).as_bytes());
    hex::encode(digest)[..ID_LEN].to_string()
}

/// Whether `id` has the shape of a server id (`id_of`): 16 lowercase hex.
/// What a peer address or a chat id names is checked by it before the
/// store is asked.
pub fn is_id(id: &str) -> bool {
    id.len() == ID_LEN && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

const COLS: &str =
    "id, kind, name, relay_url, relay_key, media_url, call_node, push_mode, source, added_at";

/// Insert or update the server of `row.relay_url`. The id and the url are
/// taken from the url whatever the row says, and `added_at` stays that of
/// the first insert (0 means now). The id it was stored under.
pub async fn put(store: &Store, row: &ServerRow) -> Result<String> {
    if row.kind != KIND_HOME && row.kind != KIND_PRIVATE {
        return Err(MessengerError::Invalid(format!(
            "server kind must be home or private, not {}",
            row.kind
        )));
    }
    if row.push_mode != PUSH_OFF && row.push_mode != PUSH_VEYDAN {
        return Err(MessengerError::Invalid(format!(
            "push mode must be off or veydan, not {}",
            row.push_mode
        )));
    }
    let url = normalize(&row.relay_url);
    if url.is_empty() {
        return Err(MessengerError::Invalid("a server needs a relay url".into()));
    }
    let id = id_of(&url);
    let added_at = if row.added_at > 0 {
        row.added_at
    } else {
        crate::now()
    };
    sqlx::query(
        "INSERT INTO msg_servers (id, kind, name, relay_url, relay_key, media_url, call_node, push_mode, source, added_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           kind = excluded.kind, name = excluded.name, relay_url = excluded.relay_url, relay_key = excluded.relay_key,
           media_url = excluded.media_url, call_node = excluded.call_node, push_mode = excluded.push_mode,
           source = excluded.source",
    )
    .bind(&id)
    .bind(&row.kind)
    .bind(&row.name)
    .bind(&url)
    .bind(&row.relay_key)
    .bind(&row.media_url)
    .bind(&row.call_node)
    .bind(&row.push_mode)
    .bind(&row.source)
    .bind(added_at)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(id)
}

pub async fn get(store: &Store, id: &str) -> Result<Option<ServerRow>> {
    sqlx::query_as::<_, ServerRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_servers WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// Every server: the home ones first, then by when they were added.
pub async fn list(store: &Store) -> Result<Vec<ServerRow>> {
    sqlx::query_as::<_, ServerRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_servers ORDER BY kind = 'private', added_at, relay_url"
    )))
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

async fn of_kind(store: &Store, kind: &str) -> Result<Vec<ServerRow>> {
    sqlx::query_as::<_, ServerRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM msg_servers WHERE kind = ? ORDER BY added_at, relay_url"
    )))
    .bind(kind)
    .fetch_all(store.pool())
    .await
    .map_err(storage)
}

/// The home set: the servers everything goes to unless a scope says otherwise.
pub async fn home(store: &Store) -> Result<Vec<ServerRow>> {
    of_kind(store, KIND_HOME).await
}

/// The servers added from a card, each its own scope.
pub async fn private(store: &Store) -> Result<Vec<ServerRow>> {
    of_kind(store, KIND_PRIVATE).await
}

/// The server whose relay is `relay_url`, in any spelling of it.
pub async fn by_relay_url(store: &Store, relay_url: &str) -> Result<Option<ServerRow>> {
    get(store, &id_of(relay_url)).await
}

/// The word for a server this device does not have: a server id, a peer
/// address or a link that names one, and the relay of a private server
/// removed on purpose.
pub const ERR_SERVER_UNKNOWN: &str = "server_unknown";

fn server_unknown() -> MessengerError {
    MessengerError::Invalid(ERR_SERVER_UNKNOWN.into())
}

/// The word for a relay of the home set (a relay of the manifest, an own
/// relay) that is the relay of a private server of this device: one relay
/// is one server, of the home set or private, never both. Followed by
/// `: <name> (<relay>)`, the server to remove first.
pub const ERR_SERVER_IS_HOME: &str = "server_is_home";

/// `server_is_home` naming private server `row`.
pub fn server_is_home(row: &ServerRow) -> MessengerError {
    let name = row.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&row.relay_url);
    MessengerError::Invalid(format!("{ERR_SERVER_IS_HOME}: {name} ({})", row.relay_url))
}

/// Whether `e` is `server_is_home`.
pub fn is_server_is_home(e: &MessengerError) -> bool {
    matches!(e, MessengerError::Invalid(m) if m.starts_with(ERR_SERVER_IS_HOME))
}

/// The private server whose relay is `relay_url`, if one is.
pub async fn private_by_relay(store: &Store, relay_url: &str) -> Result<Option<ServerRow>> {
    Ok(by_relay_url(store, relay_url).await?.filter(ServerRow::is_private))
}

/// Whether `e` is `server_unknown`.
pub fn is_unknown(e: &MessengerError) -> bool {
    matches!(e, MessengerError::Invalid(m) if m == ERR_SERVER_UNKNOWN)
}

/// The private server whose relay is `relay_url`; `None` for a relay of
/// the home set and for one that is no server of mine. The relay of a
/// private server removed on purpose (no server of mine has it, and its
/// removal is remembered, `removed_at`) is `server_unknown`: what lived on
/// it never falls back to the home set.
pub async fn private_of_relay(store: &Store, relay_url: &str) -> Result<Option<String>> {
    match by_relay_url(store, relay_url).await? {
        Some(s) => Ok(s.is_private().then_some(s.id)),
        None if removed_at(store, &id_of(relay_url)).await? > 0 => Err(server_unknown()),
        None => Ok(None),
    }
}

/// The private server group `group_id` lives on (internal/messenger-wire.md,
/// «Группы на сервере»): the server of its relay. `None` for a group of
/// the home set, of a relay that is no server of mine (every group of
/// before servers), and for a group this device does not know;
/// `server_unknown` for a group of a private server removed on purpose
/// (`group_closed`): nothing of it goes anywhere.
pub async fn of_group(store: &Store, group_id: &str) -> Result<Option<String>> {
    match crate::groups::get(store, group_id).await? {
        Some(g) if !g.relay_url.is_empty() => private_of_relay(store, &g.relay_url).await,
        _ => Ok(None),
    }
}

/// Whether group `group_id` lives on a private server removed on purpose.
/// Its removal took the group (`remove_private`); one that is here all the
/// same is closed: not listened to, nothing of it taken in or sent, and
/// never a group of the home set.
pub async fn group_closed(store: &Store, group_id: &str) -> Result<bool> {
    match of_group(store, group_id).await {
        Ok(_) => Ok(false),
        Err(e) if is_unknown(&e) => Ok(true),
        Err(e) => Err(e),
    }
}

/// Whether an event of group `group_id` that came by `relay_url` (`None`:
/// by no relay, a push) came from the group's server: a group of a
/// private server is heard from that server's relays alone, a group of
/// the home set from any relay that is no private server's. The same
/// rule for the ingress, which then leaves the event unseen, and for the
/// group service. A closed group (`group_closed`), or a relay of a
/// removed server, hears nothing.
pub async fn group_heard_on(store: &Store, group_id: &str, relay_url: Option<&str>) -> Result<bool> {
    let came_by = match relay_url {
        Some(url) => match private_of_relay(store, url).await {
            Ok(s) => s,
            Err(e) if is_unknown(&e) => return Ok(false),
            Err(e) => return Err(e),
        },
        None => None,
    };
    match of_group(store, group_id).await {
        Ok(lives_on) => Ok(lives_on == came_by),
        Err(e) if is_unknown(&e) => Ok(false),
        Err(e) => Err(e),
    }
}

/// The groups whose relay is the relay of server `id` (in any spelling of
/// it), every membership: what `of_group` gives `id` for.
async fn groups_on<'e, E>(db: E, id: &str) -> Result<Vec<String>>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let rows = sqlx::query_as::<_, (String, String)>("SELECT id, relay_url FROM msg_groups ORDER BY id")
        .fetch_all(db)
        .await
        .map_err(storage)?;
    Ok(rows
        .into_iter()
        .filter(|(_, relay)| !relay.trim().is_empty() && id_of(relay) == id)
        .map(|(g, _)| g)
        .collect())
}

/// What lives on server `id` here, for the settings: its private chats
/// (`dm:<peer>@<id>`, the rows `remove_private` takes) and the groups whose
/// relay is its relay (`of_group`), which go with it too. `(0, 0)` for a
/// server this device does not have.
pub async fn counts(store: &Store, id: &str) -> Result<(i64, i64)> {
    let Some(row) = get(store, id).await? else {
        return Ok((0, 0));
    };
    // A server id is 16 hex: no character of LIKE in it.
    let chats: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM msg_chats WHERE id LIKE 'dm:%' AND (server_id = ?1 OR id LIKE 'dm:%@' || ?1)",
    )
    .bind(&row.id)
    .fetch_one(store.pool())
    .await
    .map_err(storage)?;
    let groups = groups_on(store.pool(), &row.id).await?.len() as i64;
    Ok((chats, groups))
}

/// The chats and the groups of server `id` here: what its removal takes
/// (`remove_private`), read before it, so that the transfers of their
/// chats can be ended while their rows are still here.
pub async fn lives_on(store: &Store, id: &str) -> Result<Removed> {
    if !is_id(id) {
        return Ok(Removed::default());
    }
    let chats: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM msg_chats WHERE server_id = ?1 OR id LIKE 'dm:%@' || ?1 ORDER BY id",
    )
    .bind(id)
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    Ok(Removed { chats, groups: groups_on(store.pool(), id).await? })
}

pub async fn remove(store: &Store, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM msg_servers WHERE id = ?")
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

// ─── The push of a private server (migration 041) ──────────────────────────

/// What this device knows of the push server of a private server
/// (internal/messenger-wire.md, «Push через шлюз»).
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct ServerPush {
    pub id: String,
    /// `off` | `veydan`
    pub push_mode: String,
    /// The push server of the server, `https://…`: the card says it.
    pub push_url: Option<String>,
    /// The gateway it named in its `/v1/info`.
    pub push_gateway: Option<String>,
    /// The key it seals its pushes with, 64 hex.
    pub push_sender: Option<String>,
}

/// The push of private server `id`; `None` for a server this device does
/// not have, or a home one.
pub async fn push_of(store: &Store, id: &str) -> Result<Option<ServerPush>> {
    sqlx::query_as::<_, ServerPush>(
        "SELECT id, push_mode, push_url, push_gateway, push_sender FROM msg_servers WHERE id = ? AND kind = 'private'",
    )
    .bind(id)
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// The push server of private server `id`: `Some(url)` turns its push on
/// (mode `veydan`), `None` off. A new address forgets what the old one
/// said of its gateway and its seal. Whether a row was there to change.
pub async fn set_push(store: &Store, id: &str, push_url: Option<&str>) -> Result<bool> {
    let url = push_url.map(|u| u.trim().trim_end_matches('/').to_string()).filter(|u| !u.is_empty());
    let mode = if url.is_some() { PUSH_VEYDAN } else { PUSH_OFF };
    let done = sqlx::query(
        "UPDATE msg_servers SET push_mode = ?1,
           push_gateway = CASE WHEN push_url IS ?2 THEN push_gateway ELSE NULL END,
           push_sender = CASE WHEN push_url IS ?2 THEN push_sender ELSE NULL END,
           push_url = ?2
         WHERE id = ?3 AND kind = 'private'",
    )
    .bind(mode)
    .bind(&url)
    .bind(id)
    .execute(store.pool())
    .await
    .map_err(storage)?;
    Ok(done.rows_affected() > 0)
}

/// What the push server of private server `id` said of itself: its gateway
/// and the key of its seal; `None` forgets them.
pub async fn set_push_seal(store: &Store, id: &str, gateway: Option<&str>, sender: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE msg_servers SET push_gateway = ?, push_sender = ? WHERE id = ? AND kind = 'private'")
        .bind(gateway)
        .bind(sender)
        .bind(id)
        .execute(store.pool())
        .await
        .map_err(storage)?;
    Ok(())
}

/// The keys a sealed push may come from: the seals of the private servers
/// this device registered at. Read by the push handler, with the database
/// read-only.
pub async fn push_senders(store: &Store) -> Result<Vec<String>> {
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT push_sender FROM msg_servers
         WHERE kind = 'private' AND push_mode = 'veydan' AND push_sender IS NOT NULL",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    Ok(rows.into_iter().map(|(s,)| s).collect())
}

/// Where a private server came from: put in by hand (or a card), or taken
/// from the snapshot of my relations another device of mine published.
pub const SOURCE_HAND: &str = "hand";
pub const SOURCE_SNAPSHOT: &str = "snapshot";

/// The relay and the blob server of a private server carry this source.
const SOURCE_SERVER: &str = "server";

/// The id of the blob server row of private server `id`.
pub fn media_row_id(id: &str) -> String {
    format!("server-{id}")
}

/// Put in (or update) a private server with what goes with it: its row
/// (kind `private`), its relay (`msg_relays`, its `server_id`, the app key
/// as the relay's api key) and its blob server (`msg_media_servers`
/// `server-<id>`, the app key as `access_key`); a server without one loses
/// the row it had. A removal remembered for it (`removed_at`) is forgotten:
/// it is mine again. The one path of a server by hand and of a server taken
/// from the snapshot of my relations (internal/messenger-wire.md, «Серверы
/// на всех устройствах»). The id it was stored under.
pub async fn put_private(store: &Store, row: &ServerRow) -> Result<String> {
    // In the form the id is made of, on every device the same.
    let url = messenger_core::RelayUrl::parse(&normalize(&row.relay_url)).ok_or_else(|| {
        MessengerError::Invalid("relay url must be ws:// or wss:// with a host".into())
    })?;
    let trimmed = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(String::from)
    };
    let key = trimmed(&row.relay_key);
    let media_url = trimmed(&row.media_url).map(|u| u.trim_end_matches('/').to_string());
    // Put in again after a removal: after it, even in the same second, so
    // the removal no longer takes it on my other devices.
    let added_at = match row.added_at {
        0 => crate::now().max(removed_at(store, &id_of(url.as_str())).await? + 1),
        at => at,
    };
    let id = put(
        store,
        &ServerRow {
            id: String::new(),
            kind: KIND_PRIVATE.into(),
            name: trimmed(&row.name),
            relay_url: url.as_str().to_string(),
            relay_key: key.clone(),
            media_url: media_url.clone(),
            call_node: trimmed(&row.call_node),
            push_mode: row.push_mode.clone(),
            source: row.source.clone(),
            added_at,
        },
    )
    .await?;
    crate::relays::upsert(
        store,
        &crate::relays::RelayUpsert {
            url: url.as_str().to_string(),
            relay_id: None,
            source: SOURCE_SERVER.into(),
            regions: vec![],
            read: true,
            write: true,
            enabled: true,
            auth_type: key.as_ref().map(|_| "api_key".to_string()),
            auth_secret: key.clone(),
            server_id: Some(id.clone()),
        },
        true,
    )
    .await?;
    match media_url {
        Some(base) => {
            crate::media::upsert_server(
                store,
                &crate::media::ServerRow {
                    id: media_row_id(&id),
                    kind: crate::media::KIND_BLOSSOM.into(),
                    url: base,
                    bucket: None,
                    region: None,
                    access_key: key,
                    priority: 10,
                    enabled: true,
                    source: SOURCE_SERVER.into(),
                    created_at: 0,
                    updated_at: 0,
                    server_id: Some(id.clone()),
                },
            )
            .await?
        }
        None => crate::media::delete_server(store, &media_row_id(&id)).await?,
    }
    forget_removed(store, &id).await?;
    Ok(id)
}

/// The settings key a removal of private server `id` is remembered under.
fn removed_key(id: &str) -> String {
    format!("servers.removed@{id}")
}

/// When private server `id` was removed on this device or on another of
/// mine (my clock there); 0 when it never was, or was put in again since.
pub async fn removed_at(store: &Store, id: &str) -> Result<i64> {
    Ok(crate::settings::get(store, &removed_key(id))
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

/// Remember that private server `id` was removed at `at` (the later of two
/// removals stands), so that an older word of a device that still has it
/// does not bring it back.
pub async fn remember_removed(store: &Store, id: &str, at: i64) -> Result<()> {
    if at > removed_at(store, id).await? {
        crate::settings::set(store, &removed_key(id), &at.to_string()).await?;
    }
    Ok(())
}

async fn forget_removed(store: &Store, id: &str) -> Result<()> {
    crate::settings::delete(store, &removed_key(id)).await
}

/// Every removal remembered, `(id, removed_at)`, the newest first.
pub async fn removed(store: &Store) -> Result<Vec<(String, i64)>> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT key, value FROM msg_settings WHERE key LIKE 'servers.removed@%'",
    )
    .fetch_all(store.pool())
    .await
    .map_err(storage)?;
    let mut out: Vec<(String, i64)> = rows
        .into_iter()
        .filter_map(|(k, v)| {
            let id = k.strip_prefix("servers.removed@")?;
            Some((id.to_string(), v.parse().ok()?))
        })
        .filter(|(id, _)| is_id(id))
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(out)
}

/// The user removes private server `id` here (`remove_private`), and the
/// removal is remembered after its `added_at` even when the clock of the
/// device that added it runs ahead, so it takes the server on every device
/// of mine. What went; `None` for a server that is no private server of
/// this device.
pub async fn remove_by_me(store: &Store, id: &str, now: i64) -> Result<Option<Removed>> {
    let Some(row) = get(store, id).await?.filter(ServerRow::is_private) else {
        return Ok(None);
    };
    let removed = remove_private(store, id).await?;
    remember_removed(store, id, now.max(row.added_at + 1)).await?;
    Ok(removed)
}

/// What went with a private server (`remove_private`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Removed {
    /// Its private chats, `dm:<peer>@<id>`.
    pub chats: Vec<String>,
    /// Its groups: the ids (their chats are `group:<id>`).
    pub groups: Vec<String>,
}

impl Removed {
    pub fn is_empty(&self) -> bool {
        self.chats.is_empty() && self.groups.is_empty()
    }
}

/// The rows of a chat, by its column `chat_id` (`{CHAT}`, a condition the
/// caller binds as `?1`) or by the messages of it.
const OF_A_CHAT: [&str; 12] = [
    "DELETE FROM msg_dm_delivered WHERE message_id IN (SELECT id FROM msg_messages WHERE chat_id {CHAT})",
    "DELETE FROM msg_own_hidden WHERE message_id IN (SELECT id FROM msg_messages WHERE chat_id {CHAT})",
    "DELETE FROM msg_reactions WHERE chat_id {CHAT}",
    "DELETE FROM msg_dm_held WHERE chat_id {CHAT}",
    "DELETE FROM msg_transfers WHERE chat_id {CHAT}",
    "DELETE FROM msg_messages WHERE chat_id {CHAT}",
    "DELETE FROM msg_chat_deleted_ids WHERE chat_id {CHAT}",
    "DELETE FROM msg_chat_cleared WHERE chat_id {CHAT}",
    "DELETE FROM msg_chat_prefs WHERE chat_id {CHAT}",
    "DELETE FROM msg_own_read WHERE chat_id {CHAT}",
    "DELETE FROM msg_peer_read WHERE chat_id {CHAT}",
    "DELETE FROM msg_calls WHERE chat_id {CHAT}",
];

async fn delete_chat_rows(tx: &mut sqlx::SqliteConnection, condition: &str, bound: &str) -> Result<()> {
    for sql in OF_A_CHAT {
        sqlx::query(sqlx::AssertSqlSafe(sql.replace("{CHAT}", condition)))
            .bind(bound)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    Ok(())
}

/// Group `group_id` forgotten in `tx` as `GroupService::forget` forgets a
/// group I left, and more, since nothing of it will come again: its rows
/// (log, keys list, pending, invitations, requests, marks of moderation,
/// picture), its events, its settings (`group.<id>.…`), its chat with
/// every row of it, and what the outbox still had to send to it. The ids
/// of its keys are put in `groups::SECRETS_OWED`: the keys themselves are
/// in the secret store, which the group service empties
/// (`GroupService::forget_gone`).
async fn forget_group(tx: &mut sqlx::SqliteConnection, group_id: &str) -> Result<()> {
    let keys: Vec<String> = sqlx::query_scalar("SELECT key_id FROM msg_group_keys WHERE group_id = ? ORDER BY key_id")
        .bind(group_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage)?;
    if !keys.is_empty() {
        let mut owed = crate::groups::secrets_owed_in(&mut *tx).await?;
        owed.extend(keys.into_iter().map(|k| (group_id.to_string(), k)));
        crate::groups::set_secrets_owed_in(&mut *tx, &owed).await?;
    }
    for sql in [
        "DELETE FROM msg_group_ops WHERE group_id = ?1",
        "DELETE FROM msg_group_keys WHERE group_id = ?1",
        "DELETE FROM msg_group_pending WHERE group_id = ?1",
        "DELETE FROM msg_group_invites WHERE group_id = ?1",
        "DELETE FROM msg_group_requests WHERE group_id = ?1",
        "DELETE FROM msg_group_marks WHERE group_id = ?1",
        "DELETE FROM msg_group_avatar WHERE group_id = ?1",
        "DELETE FROM msg_groups WHERE id = ?1",
        "DELETE FROM msg_outbox WHERE state != 'published'
           AND json_extract(outbound_json, '$.scope.scope') = 'group' AND json_extract(outbound_json, '$.scope.id') = ?1",
    ] {
        sqlx::query(sql).bind(group_id).execute(&mut *tx).await.map_err(storage)?;
    }
    // A group id is hex (`events_raw::forget_tagged` asks the same): no
    // character of LIKE in it.
    if group_id.len() >= 32 && group_id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        sqlx::query("DELETE FROM msg_events_raw WHERE kind = 9 AND raw_json LIKE ?")
            .bind(format!("%\"{group_id}\"%"))
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        sqlx::query("DELETE FROM msg_settings WHERE key LIKE ?")
            .bind(format!("group.{group_id}.%"))
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    let chat = crate::groups::group_chat_id(group_id);
    delete_chat_rows(tx, "= ?1", &chat).await?;
    sqlx::query("DELETE FROM msg_chats WHERE id = ?")
        .bind(&chat)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    Ok(())
}

/// The invitations to a group of server `id` that has no row here (not
/// answered yet): by the relay each names.
async fn invites_on(tx: &mut sqlx::SqliteConnection, id: &str) -> Result<Vec<String>> {
    let rows = sqlx::query_as::<_, (String, String)>("SELECT invite_id, payload_json FROM msg_group_invites")
        .fetch_all(&mut *tx)
        .await
        .map_err(storage)?;
    Ok(rows
        .into_iter()
        .filter(|(_, payload)| {
            serde_json::from_str::<serde_json::Value>(payload)
                .ok()
                .and_then(|v| v.get("relay").and_then(|r| r.as_str()).map(id_of))
                .is_some_and(|of| of == id)
        })
        .map(|(invite, _)| invite)
        .collect())
}

/// The events of `msg_events_raw` that came by the relay of server `id`
/// (live or as history; the relay in any spelling of it), forgotten.
async fn forget_events_of(tx: &mut sqlx::SqliteConnection, id: &str) -> Result<()> {
    let relays: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT json_extract(source_json, '$.url') FROM msg_events_raw
         WHERE json_extract(source_json, '$.url') IS NOT NULL",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(storage)?;
    for relay in relays.into_iter().filter(|r| id_of(r) == id) {
        sqlx::query("DELETE FROM msg_events_raw WHERE json_extract(source_json, '$.url') = ?")
            .bind(relay)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    Ok(())
}

/// Remove private server `id` and everything of it on this device: its
/// row, its relay, its blob server, its private chats (`dm:<peer>@<id>`)
/// with their messages, reactions, receipts, held rows, choices, deletion
/// marks and calls, the relations and inbox lists of its peers, its
/// cursors and its settings (`…@<id>`), and what the outbox still had to
/// send there (nothing else can carry it). Its groups go too, each as a
/// group I left is forgotten (`forget_group`), whatever my membership:
/// their relay is no server of mine any more, and a group of it must not
/// become a group of the home set; nothing is published, and the other
/// members keep the group. So do the invitations to a group of it. No
/// tombstone of the chats: the server is gone, and put in again it brings
/// the history of its private chats back (a group comes back only as any
/// group comes, by a welcome). A home
/// server, or one this device does not have, is left alone (`None`). What
/// went.
pub async fn remove_private(store: &Store, id: &str) -> Result<Option<Removed>> {
    if !is_id(id) {
        return Ok(None);
    }
    match get(store, id).await? {
        Some(row) if row.is_private() => {}
        _ => return Ok(None),
    }
    // A server id is 16 hex: no character of LIKE in it.
    let chats_like = format!("dm:%@{id}");
    // A writer from the start: it reads before it deletes, and a reader
    // that would write after another connection wrote fails at once
    // (`SQLITE_BUSY_SNAPSHOT`, no busy wait) — the ingress writes all the
    // while.
    let mut tx = store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
    let chats: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM msg_chats WHERE server_id = ?1 OR id LIKE ?2 ORDER BY id",
    )
    .bind(id)
    .bind(&chats_like)
    .fetch_all(&mut *tx)
    .await
    .map_err(storage)?;
    let groups = groups_on(&mut *tx, id).await?;
    for g in &groups {
        forget_group(&mut tx, g).await?;
    }
    for invite in invites_on(&mut tx, id).await? {
        sqlx::query("DELETE FROM msg_group_invites WHERE invite_id = ?")
            .bind(invite)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    delete_chat_rows(&mut tx, "LIKE ?1", &chats_like).await?;
    // The events that came by its relay are no longer seen: put in again,
    // the server's history comes back whole, its wraps not skipped as
    // known by the ingress nor left out of a reconciliation.
    forget_events_of(&mut tx, id).await?;
    for sql in [
        "DELETE FROM msg_chats WHERE server_id = ?1",
        "DELETE FROM msg_dm_relations WHERE server_id = ?1",
        "DELETE FROM msg_dm_routes WHERE server_id = ?1",
        "DELETE FROM msg_sync_cursors WHERE relay_url = ?1",
        "DELETE FROM msg_relays WHERE server_id = ?1",
        "DELETE FROM msg_media_servers WHERE server_id = ?1",
        "DELETE FROM msg_settings WHERE key LIKE '%@' || ?1",
        "DELETE FROM msg_outbox WHERE state != 'published'
           AND json_extract(outbound_json, '$.scope.scope') = 'server' AND json_extract(outbound_json, '$.scope.id') = ?1",
        "DELETE FROM msg_servers WHERE id = ?1 AND kind = 'private'",
    ] {
        sqlx::query(sql)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    sqlx::query("DELETE FROM msg_chats WHERE id LIKE ?1")
        .bind(&chats_like)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(Some(Removed { chats, groups }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(url: &str, kind: &str) -> ServerRow {
        ServerRow {
            id: String::new(),
            kind: kind.into(),
            name: None,
            relay_url: url.into(),
            relay_key: None,
            media_url: None,
            call_node: None,
            push_mode: PUSH_OFF.into(),
            source: None,
            added_at: 0,
        }
    }

    #[test]
    fn the_id_is_the_hash_of_the_normal_url() {
        let want = hex::encode(Sha256::digest(b"wss://relay.example"))[..16].to_string();
        assert_eq!(id_of("wss://relay.example"), want);
        assert_eq!(
            id_of(" WSS://Relay.Example/ "),
            want,
            "one server, whatever the spelling"
        );
        assert_eq!(id_of("wss://relay.example").len(), 16);
        assert_ne!(id_of("wss://relay.example"), id_of("wss://other.example"));
    }

    /// The push of a private server (migration 041): on with an address,
    /// off without; a new address forgets what the old one said of its
    /// gateway and its seal; the seals are those of servers whose push is on.
    #[tokio::test]
    async fn the_push_of_a_private_server() {
        let s = Store::open_in_memory().await.unwrap();
        let home = put(&s, &row("wss://home.example", KIND_HOME)).await.unwrap();
        let id = put_private(&s, &row("wss://mine.example", KIND_PRIVATE)).await.unwrap();
        assert!(!set_push(&s, &home, Some("https://x.example")).await.unwrap(), "a home server has none");
        assert!(push_of(&s, &home).await.unwrap().is_none());

        assert!(set_push(&s, &id, Some(" https://push.mine.example/ ")).await.unwrap());
        set_push_seal(&s, &id, Some("https://gw.example"), Some(&"ab".repeat(32))).await.unwrap();
        let p = push_of(&s, &id).await.unwrap().unwrap();
        assert_eq!((p.push_mode.as_str(), p.push_url.as_deref()), (PUSH_VEYDAN, Some("https://push.mine.example")));
        assert_eq!(push_senders(&s).await.unwrap(), vec!["ab".repeat(32)]);

        // The same address again keeps the seal; another forgets it.
        set_push(&s, &id, Some("https://push.mine.example")).await.unwrap();
        assert_eq!(push_senders(&s).await.unwrap().len(), 1);
        set_push(&s, &id, Some("https://other-push.mine.example")).await.unwrap();
        let p = push_of(&s, &id).await.unwrap().unwrap();
        assert_eq!((p.push_gateway, p.push_sender), (None, None));

        // Off: no address, no seal taken.
        set_push_seal(&s, &id, Some("https://gw.example"), Some(&"cd".repeat(32))).await.unwrap();
        set_push(&s, &id, None).await.unwrap();
        let p = push_of(&s, &id).await.unwrap().unwrap();
        assert_eq!((p.push_mode.as_str(), p.push_url), (PUSH_OFF, None));
        assert!(push_senders(&s).await.unwrap().is_empty());

        // A put of the row again (a card scanned anew) says the mode; the
        // address stays for `set_push` to turn it on again.
        set_push(&s, &id, Some("https://push.mine.example")).await.unwrap();
        put_private(&s, &row("wss://mine.example", KIND_PRIVATE)).await.unwrap();
        let p = push_of(&s, &id).await.unwrap().unwrap();
        assert_eq!((p.push_mode.as_str(), p.push_url.as_deref()), (PUSH_OFF, Some("https://push.mine.example")));
    }

    #[tokio::test]
    async fn put_get_list_remove() {
        let s = Store::open_in_memory().await.unwrap();
        let home_id = put(&s, &row("wss://Home.example/", KIND_HOME))
            .await
            .unwrap();
        assert_eq!(home_id, id_of("wss://home.example"));
        let mut private_row = row("wss://mine.example", KIND_PRIVATE);
        private_row.name = Some("Мой сервер".into());
        private_row.relay_key = Some("k".repeat(16));
        private_row.added_at = 5;
        let private_id = put(&s, &private_row).await.unwrap();

        let got = get(&s, &home_id).await.unwrap().unwrap();
        assert_eq!(
            got.relay_url, "wss://home.example",
            "stored in its normal form"
        );
        assert!(got.added_at > 0, "0 means now");
        let first_added = got.added_at;

        let list = list(&s).await.unwrap();
        assert_eq!(
            list.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(),
            vec![KIND_HOME, KIND_PRIVATE],
            "home first"
        );
        assert_eq!(home(&s).await.unwrap().len(), 1);
        let private_rows = private(&s).await.unwrap();
        assert_eq!(private_rows.len(), 1);
        assert!(private_rows[0].is_private());
        assert_eq!(private_rows[0].name.as_deref(), Some("Мой сервер"));

        assert_eq!(
            by_relay_url(&s, "wss://MINE.example/")
                .await
                .unwrap()
                .unwrap()
                .id,
            private_id
        );
        assert!(by_relay_url(&s, "wss://nobody.example")
            .await
            .unwrap()
            .is_none());

        // Put again: updated in place, still first added when it was.
        let mut again = row("wss://home.example", KIND_HOME);
        again.push_mode = PUSH_VEYDAN.into();
        again.added_at = first_added + 100;
        put(&s, &again).await.unwrap();
        let got = get(&s, &home_id).await.unwrap().unwrap();
        assert_eq!(
            (got.push_mode.as_str(), got.added_at),
            (PUSH_VEYDAN, first_added)
        );
        assert_eq!(super::list(&s).await.unwrap().len(), 2);

        remove(&s, &private_id).await.unwrap();
        assert!(get(&s, &private_id).await.unwrap().is_none());
        assert!(private(&s).await.unwrap().is_empty());
    }

    /// A database of before servers, with relays, a blob server and
    /// cursors in it, taken through migration 039: every row is of the home
    /// set, and the cursors under `*` still answer for it.
    #[tokio::test]
    async fn the_rows_of_before_servers_are_the_home_set() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
        let options = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        let mut before = sqlx::migrate!("./migrations");
        before.migrations = before
            .migrations
            .iter()
            .filter(|m| m.version < 39)
            .cloned()
            .collect::<Vec<_>>()
            .into();
        before.run(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO msg_relays (url, relay_id, source, regions_json, read, write, enabled, created_at, updated_at)
             VALUES ('wss://home.example', 'h', 'manifest', '[]', 1, 1, 1, 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO msg_media_servers (id, kind, url, priority, enabled, source, created_at, updated_at)
             VALUES ('m', 'blossom', 'https://media.example', 10, 1, 'manifest', 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO msg_sync_cursors (scope, relay_url, last_event_at) VALUES ('dm-inbox', '*', 1234)")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let s = Store { pool };
        let relay = crate::relays::get(&s, "wss://home.example")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (relay.relay_id.as_deref(), relay.server_id),
            (Some("h"), None)
        );
        let media = crate::media::server(&s, "m").await.unwrap().unwrap();
        assert_eq!((media.server_id, media.access_key), (None, None));
        assert_eq!(crate::media::home_servers(&s).await.unwrap().len(), 1);
        assert_eq!(
            crate::cursors::get(&s, "dm-inbox", crate::cursors::of_server(None))
                .await
                .unwrap(),
            Some(1234)
        );
        assert!(
            list(&s).await.unwrap().is_empty(),
            "the runtime makes the home rows, not the migration"
        );
    }

    fn message(id: &str, chat: &str) -> crate::messages::NewMessage {
        crate::messages::NewMessage {
            id: id.into(),
            chat_id: chat.into(),
            wire_id: None,
            direction: "in".into(),
            status: "received".into(),
            content_type: "text".into(),
            text: Some("hi".into()),
            envelope_json: "{}".into(),
            sender_pubkey: "p".into(),
            reply_to_id: None,
            target_id: None,
            created_at: 10,
            is_hidden: false,
            outbox_local_id: None,
            media_json: None,
        }
    }

    /// A private server comes with its relay and its blob server, and goes
    /// with them, its chats and all of them; the home chat with the same
    /// person stays. A removal is remembered, and a new put forgets it.
    #[tokio::test]
    async fn a_private_server_comes_and_goes_with_what_is_its() {
        let s = Store::open_in_memory().await.unwrap();
        let mut r = row(" wss://Mine.example/ ", KIND_PRIVATE);
        r.relay_key = Some(" k3y ".into());
        r.media_url = Some("https://media.mine.example/".into());
        r.name = Some("S1".into());
        r.source = Some(SOURCE_HAND.into());
        let id = put_private(&s, &r).await.unwrap();
        assert_eq!(id, id_of("wss://mine.example"));
        let got = get(&s, &id).await.unwrap().unwrap();
        assert_eq!(
            (
                got.kind.as_str(),
                got.relay_key.as_deref(),
                got.media_url.as_deref()
            ),
            (
                KIND_PRIVATE,
                Some("k3y"),
                Some("https://media.mine.example")
            )
        );
        let relay = crate::relays::get(&s, "wss://mine.example")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (relay.server_id.as_deref(), relay.auth_secret.as_deref()),
            (Some(id.as_str()), Some("k3y"))
        );
        let media = crate::media::server(&s, &media_row_id(&id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (media.server_id.as_deref(), media.access_key.as_deref()),
            (Some(id.as_str()), Some("k3y"))
        );
        assert!(put_private(&s, &row("https://not.a.relay", KIND_PRIVATE))
            .await
            .is_err());

        // Its chat, the home chat with the same person, and what is theirs.
        let private_chat = crate::chats::ensure_dm(&s, "p", Some(&id))
            .await
            .unwrap()
            .id;
        let home_chat = crate::chats::ensure_dm(&s, "p", None).await.unwrap().id;
        crate::messages::insert(&s, &message("m1", &private_chat))
            .await
            .unwrap();
        crate::messages::insert(&s, &message("m2", &home_chat))
            .await
            .unwrap();
        crate::cursors::advance(&s, "dm-inbox", &id, 5)
            .await
            .unwrap();
        crate::settings::set(&s, &format!("dm.relays_published@{id}"), "x")
            .await
            .unwrap();
        crate::settings::set(&s, "dm.relays_published", "home")
            .await
            .unwrap();

        assert_eq!(
            remove_by_me(&s, &id_of("wss://other.example"), 100)
                .await
                .unwrap(),
            None,
            "not a server of mine"
        );
        let gone = remove_by_me(&s, &id, 1).await.unwrap().unwrap();
        assert_eq!(gone, Removed { chats: vec![private_chat.clone()], groups: vec![] });
        assert!(get(&s, &id).await.unwrap().is_none());
        assert!(crate::relays::get(&s, "wss://mine.example")
            .await
            .unwrap()
            .is_none());
        assert!(crate::media::server(&s, &media_row_id(&id))
            .await
            .unwrap()
            .is_none());
        assert!(crate::chats::get(&s, &private_chat)
            .await
            .unwrap()
            .is_none());
        assert!(crate::messages::get(&s, "m1").await.unwrap().is_none());
        assert!(crate::chats::get(&s, &home_chat).await.unwrap().is_some());
        assert!(crate::messages::get(&s, "m2").await.unwrap().is_some());
        assert_eq!(
            crate::cursors::get(&s, "dm-inbox", &id).await.unwrap(),
            None
        );
        assert_eq!(
            crate::settings::get(&s, &format!("dm.relays_published@{id}"))
                .await
                .unwrap(),
            None
        );
        assert!(crate::settings::get(&s, "dm.relays_published")
            .await
            .unwrap()
            .is_some());
        let at = removed_at(&s, &id).await.unwrap();
        assert!(at > got.added_at, "after it was added, whatever the clock");
        assert_eq!(removed(&s).await.unwrap(), vec![(id.clone(), at)]);
        remember_removed(&s, &id, at - 1).await.unwrap();
        assert_eq!(removed_at(&s, &id).await.unwrap(), at, "the later stands");

        // Put in again: after the removal, which is forgotten.
        let again = put_private(&s, &r).await.unwrap();
        assert!(get(&s, &again).await.unwrap().unwrap().added_at > at);
        assert_eq!(removed_at(&s, &id).await.unwrap(), 0);
        assert!(removed(&s).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn what_lives_on_a_server_is_counted() {
        let s = Store::open_in_memory().await.unwrap();
        let id = put_private(&s, &row("wss://mine.example", KIND_PRIVATE)).await.unwrap();
        assert_eq!(counts(&s, &id).await.unwrap(), (0, 0));
        assert_eq!(counts(&s, &id_of("wss://other.example")).await.unwrap(), (0, 0), "not a server of mine");
        crate::chats::ensure_dm(&s, "p", Some(&id)).await.unwrap();
        crate::chats::ensure_dm(&s, "q", Some(&id)).await.unwrap();
        crate::chats::ensure_dm(&s, "p", None).await.unwrap();
        let group = |gid: &str, relay: &str| crate::groups::GroupRow {
            id: gid.into(),
            kind: "private".into(),
            name: "Team".into(),
            about: String::new(),
            picture: String::new(),
            relay_url: relay.into(),
            owner: "o".into(),
            membership: "joined".into(),
            my_role: None,
            members: 1,
            link_epoch: 0,
            created_at: 0,
            updated_at: 0,
            needs_update: false,
        };
        crate::groups::upsert(&s, &group("g1", "wss://Mine.example/")).await.unwrap();
        crate::groups::upsert(&s, &group("g2", "wss://home.example")).await.unwrap();
        assert_eq!(counts(&s, &id).await.unwrap(), (2, 1), "its two chats and its group, not the home ones");
        remove_by_me(&s, &id, 1).await.unwrap();
        assert_eq!(counts(&s, &id).await.unwrap(), (0, 0));
    }

    fn group_row(gid: &str, relay: &str, membership: &str) -> crate::groups::GroupRow {
        crate::groups::GroupRow {
            id: gid.into(),
            kind: "private".into(),
            name: "Team".into(),
            about: String::new(),
            picture: String::new(),
            relay_url: relay.into(),
            owner: "o".into(),
            membership: membership.into(),
            my_role: None,
            members: 1,
            link_epoch: 0,
            created_at: 0,
            updated_at: 0,
            needs_update: false,
        }
    }

    async fn count(s: &Store, sql: &str, bind: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
            .bind(bind)
            .fetch_one(s.pool())
            .await
            .unwrap()
    }

    /// Stage Э3 of private servers, the end of it: a removed server takes
    /// its groups (every membership) as a group I left is forgotten — rows,
    /// events, settings, chat and messages, what the outbox had for them —
    /// and the invitations to a group of it; the ids of their keys are owed
    /// to the secret store. A group of the home set stays. A group of the
    /// removed server that is here all the same is closed, never home.
    #[tokio::test]
    async fn a_removed_server_takes_its_groups_with_it() {
        use messenger_core::outbound::WireEvent;
        use messenger_core::{EventId, Outbound, Scope};
        let s = Store::open_in_memory().await.unwrap();
        let id = put_private(&s, &row("wss://mine.example", KIND_PRIVATE)).await.unwrap();
        let (g1, g2, home_g) = ("1".repeat(64), "2".repeat(64), "3".repeat(64));
        crate::groups::upsert(&s, &group_row(&g1, "wss://Mine.example/", "joined")).await.unwrap();
        crate::groups::upsert(&s, &group_row(&g2, "wss://mine.example", "requested")).await.unwrap();
        crate::groups::upsert(&s, &group_row(&home_g, "wss://home.example", "joined")).await.unwrap();
        assert_eq!(of_group(&s, &g1).await.unwrap(), Some(id.clone()));
        for (g, key) in [(&g1, "k1"), (&g1, "k2"), (&home_g, "kh")] {
            crate::groups::add_key(&s, g, key).await.unwrap();
        }
        crate::groups::insert_op(&s, &g1, "op1", "{}").await.unwrap();
        for g in [&g1, &home_g] {
            let chat = crate::groups::group_chat_id(g);
            crate::messages::insert(&s, &message(&format!("m-{g}"), &chat)).await.unwrap();
            sqlx::query(
                "INSERT INTO msg_events_raw (event_id, kind, pubkey, created_at, received_at, source_json, raw_json)
                 VALUES (?, 9, 'p', 1, 1, '{}', ?)",
            )
            .bind(format!("e{}", &g[1..]))
            .bind(format!("{{\"tags\":[[\"h\",\"{g}\"]]}}"))
            .execute(s.pool())
            .await
            .unwrap();
            crate::settings::set(&s, &format!("group.{g}.oldest_fetched"), "5 0").await.unwrap();
            let out = Outbound::PublishScoped {
                scope: Scope::Group { id: g.to_string() },
                event: WireEvent { id: EventId::parse(&"a".repeat(64)).unwrap(), json: serde_json::json!({}) },
            };
            crate::outbox::enqueue(&s, &format!("o-{g}"), &out, 1, None).await.unwrap();
        }
        // An invitation to a group of the server that has no row here.
        let invite = |invite_id: &str, relay: &str| crate::groups::InviteRow {
            invite_id: invite_id.into(),
            group_id: "4".repeat(64),
            direction: "in".into(),
            peer: "p".into(),
            status: "received".into(),
            payload_json: serde_json::json!({ "relay": relay }).to_string(),
            created_at: 1,
            expires_at: i64::MAX,
            updated_at: 1,
        };
        crate::groups::insert_invite(&s, &invite("i-mine", "wss://MINE.example")).await.unwrap();
        crate::groups::insert_invite(&s, &invite("i-home", "wss://home.example")).await.unwrap();
        assert_eq!(counts(&s, &id).await.unwrap(), (0, 2));
        assert_eq!(lives_on(&s, &id).await.unwrap(), Removed { chats: vec![], groups: vec![g1.clone(), g2.clone()] });

        let gone = remove_by_me(&s, &id, 1).await.unwrap().unwrap();
        assert_eq!(gone, Removed { chats: vec![], groups: vec![g1.clone(), g2.clone()] });
        for g in [&g1, &g2] {
            assert!(crate::groups::get(&s, g).await.unwrap().is_none());
            assert!(crate::chats::get(&s, &crate::groups::group_chat_id(g)).await.unwrap().is_none());
            assert!(crate::settings::get(&s, &format!("group.{g}.oldest_fetched")).await.unwrap().is_none());
        }
        assert!(crate::messages::get(&s, &format!("m-{g1}")).await.unwrap().is_none());
        assert_eq!(count(&s, "SELECT COUNT(*) FROM msg_group_ops WHERE group_id = ?", &g1).await, 0);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM msg_group_keys WHERE group_id = ?", &g1).await, 0);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM msg_events_raw WHERE raw_json LIKE '%' || ? || '%'", &g1).await, 0);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM msg_outbox WHERE local_id = ?", &format!("o-{g1}")).await, 0);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM msg_group_invites WHERE invite_id = ?", "i-mine").await, 0);
        let mut owed = crate::groups::secrets_owed(&s).await.unwrap();
        owed.sort();
        assert_eq!(owed, vec![(g1.clone(), "k1".to_string()), (g1.clone(), "k2".to_string())]);
        crate::groups::secrets_forgotten(&s, &owed[..1]).await.unwrap();
        assert_eq!(crate::groups::secrets_owed(&s).await.unwrap(), vec![(g1.clone(), "k2".to_string())]);

        // The home group and all of it stay.
        assert!(crate::groups::get(&s, &home_g).await.unwrap().is_some());
        assert!(crate::messages::get(&s, &format!("m-{home_g}")).await.unwrap().is_some());
        assert!(crate::settings::get(&s, &format!("group.{home_g}.oldest_fetched")).await.unwrap().is_some());
        assert_eq!(count(&s, "SELECT COUNT(*) FROM msg_group_keys WHERE group_id = ?", &home_g).await, 1);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM msg_events_raw WHERE raw_json LIKE '%' || ? || '%'", &home_g).await, 1);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM msg_outbox WHERE local_id = ?", &format!("o-{home_g}")).await, 1);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM msg_group_invites WHERE invite_id = ?", "i-home").await, 1);
        assert_eq!(of_group(&s, &home_g).await.unwrap(), None);

        // A group of the removed server here all the same (a row of a
        // build before this one): closed, never home, hearing nothing.
        crate::groups::upsert(&s, &group_row(&g1, "wss://mine.example", "joined")).await.unwrap();
        assert!(is_unknown(&of_group(&s, &g1).await.unwrap_err()));
        assert!(is_unknown(&private_of_relay(&s, "wss://MINE.example/").await.unwrap_err()));
        assert!(group_closed(&s, &g1).await.unwrap());
        assert!(!group_closed(&s, &home_g).await.unwrap());
        assert!(!group_heard_on(&s, &g1, None).await.unwrap());
        assert!(!group_heard_on(&s, &g1, Some("wss://home.example")).await.unwrap());
        assert!(!group_heard_on(&s, &home_g, Some("wss://mine.example")).await.unwrap(), "nor by its relay");
        assert!(group_heard_on(&s, &home_g, Some("wss://home.example")).await.unwrap());
        // A relay that never was a server of mine is still the home set's.
        assert_eq!(private_of_relay(&s, "wss://never.example").await.unwrap(), None);

        // Put in again: the server is mine, and so is the group's relay.
        put_private(&s, &row("wss://mine.example", KIND_PRIVATE)).await.unwrap();
        assert_eq!(of_group(&s, &g1).await.unwrap(), Some(id.clone()));
        assert!(!group_closed(&s, &g1).await.unwrap());
    }

    #[tokio::test]
    async fn a_row_of_no_known_kind_or_push_mode_is_refused() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(put(&s, &row("wss://a.example", "public")).await.is_err());
        let mut r = row("wss://a.example", KIND_HOME);
        r.push_mode = "fcm".into();
        assert!(put(&s, &r).await.is_err());
        assert!(put(&s, &row("  ", KIND_HOME)).await.is_err());
        assert!(list(&s).await.unwrap().is_empty());
    }

    /// A removed server's events are seen no more, so the server put in
    /// again brings its history back; those of the home set and of
    /// another server stay seen.
    #[tokio::test]
    async fn a_removal_forgets_the_events_that_came_by_the_server() {
        use messenger_core::{EventId, EventSource, PubKey, RawEvent, RelayUrl, Timestamp};
        let s = Store::open_in_memory().await.unwrap();
        let id = put_private(&s, &row("wss://s1.example", KIND_PRIVATE)).await.unwrap();
        put_private(&s, &row("wss://s2.example", KIND_PRIVATE)).await.unwrap();
        let seen = |n: u64, source: EventSource| RawEvent {
            id: EventId::parse(&format!("{n:064x}")).unwrap(),
            kind: 1059,
            pubkey: PubKey::parse(&"b".repeat(64)).unwrap(),
            created_at: Timestamp(1_000),
            json: serde_json::json!({}),
            source,
        };
        let url = |u: &str| RelayUrl::parse(u).unwrap();
        for (n, source) in [
            (1, EventSource::Relay { url: url("wss://s1.example") }),
            (2, EventSource::Sync { url: url("wss://S1.example/") }),
            (3, EventSource::Relay { url: url("wss://home.example") }),
            (4, EventSource::Relay { url: url("wss://s2.example") }),
            (5, EventSource::Server { id: "push".into() }),
        ] {
            assert!(crate::events_raw::insert_if_new(&s, &seen(n, source)).await.unwrap());
        }
        remove_by_me(&s, &id, 10).await.unwrap().unwrap();
        let mut known = Vec::new();
        for n in 1..=5u64 {
            known.push(crate::events_raw::contains(&s, &EventId::parse(&format!("{n:064x}")).unwrap()).await.unwrap());
        }
        assert_eq!(known, vec![false, false, true, true, true], "what S1 sent, in any spelling of it, and nothing else");
    }
}

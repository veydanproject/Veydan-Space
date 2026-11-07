// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Whose data the messenger keeps, and the wipe of it.
//!
//! The chats, the messages, the contacts, the groups and the rest are the
//! data of one identity. When its key goes (a reset of the lock, the demo
//! data, «delete the key») they stay, so that the same key imported again
//! finds them; another key must never see them. `KEY_OWNER` keeps the
//! public key of the identity they belong to, and outlives the identity
//! row: the next create or import compares (`IdentityService::persist`).
//!
//! What is wiped is what belongs to the account: every table but those of
//! the device ([`KEPT_TABLES`]: the identity row, the relays, the media
//! servers, the private servers, the ban lists of the network) and the
//! settings of the account ([`ACCOUNT_SETTINGS`]: by the area of the key).
//! The settings of the device — the servers' mode, the bridge, the
//! manifest, notifications, push, calls — stay.

use crate::{identity, settings, storage, Store};
use messenger_core::Result;

/// The hex public key of the identity whose data the store holds.
pub const KEY_OWNER: &str = "account.owner";

/// The tables of the device, which a wipe leaves. Every other table of the
/// messenger is the account's (`wipe`; a test holds every table to one list
/// or the other).
pub const KEPT_TABLES: [&str; 7] = [
    "msg_settings",
    "msg_identity",
    "msg_relays",
    "msg_media_servers",
    "msg_servers",
    "msg_banned_keys",
    "msg_banned_groups",
];

/// The tables of the account, emptied by `wipe`.
pub const ACCOUNT_TABLES: [&str; 47] = [
    "msg_events_raw",
    "msg_outbox",
    "msg_sync_cursors",
    "msg_profiles",
    "msg_private_contacts",
    "msg_follows",
    "msg_chats",
    "msg_messages",
    "msg_dm_routes",
    "msg_dm_relations",
    "msg_transfers",
    "msg_groups",
    "msg_group_ops",
    "msg_group_keys",
    "msg_group_pending",
    "msg_group_invites",
    "msg_group_requests",
    "msg_group_marks",
    "msg_group_avatar",
    "msg_link_previews",
    "msg_own_read",
    "msg_own_hidden",
    "msg_peer_read",
    "msg_dm_delivered",
    "msg_reactions",
    "msg_emoji_usage",
    "msg_presence_keys",
    "msg_presence",
    "msg_presence_told",
    "msg_own_private",
    "msg_own_avatar",
    "msg_avatar_cache",
    "msg_contact_private",
    "msg_calls",
    "msg_call_early",
    "msg_chat_cleared",
    "msg_chat_deleted_ids",
    "msg_chat_prefs",
    "msg_dm_held",
    "msg_dm_invites",
    "msg_dm_invite_uses",
    "msg_reports_outbox",
    "msg_archive_events",
    "msg_archive_state",
    "msg_archive_queue",
    "msg_archive_forgotten",
    "msg_groups_forgotten",
];

/// The areas of the settings of the account (`<area>.<name>`): what the
/// account published or was told, its cursors, its preferences that my
/// devices agree on (privacy, the UI's, emoji). `groups.secrets_owed` is
/// of them but stays: it names the keys of the groups still to be taken
/// out of the secret store, those of the wiped groups among them.
pub const ACCOUNT_SETTINGS: [&str; 16] = [
    "avatar", "contacts", "dm", "emoji", "group", "groups", "group_call", "invite", "invites", "own_profile",
    "presence", "privacy", "reactions", "relations", "session", "ui_prefs",
];

/// Whose data the store holds: the identity recorded (`KEY_OWNER`), the
/// identity row, or, in a store from before the record, the author of a
/// message sent from here. `None`: nothing tells.
pub async fn owner(store: &Store) -> Result<Option<String>> {
    if let Some(owner) = settings::get(store, KEY_OWNER).await?.filter(|v| !v.is_empty()) {
        return Ok(Some(owner));
    }
    if let Some(row) = identity::get(store).await? {
        return Ok(Some(row.pubkey_hex));
    }
    sqlx::query_scalar::<_, String>(
        "SELECT sender_pubkey FROM msg_messages WHERE direction = 'out' AND sender_pubkey <> '' LIMIT 1",
    )
    .fetch_optional(store.pool())
    .await
    .map_err(storage)
}

/// The tables whose rows a person sees as the data of an account: chats,
/// messages, contacts, relations, groups and their invitations, calls.
pub const SEEN_TABLES: [&str; 9] = [
    "msg_chats",
    "msg_messages",
    "msg_private_contacts",
    "msg_follows",
    "msg_dm_relations",
    "msg_groups",
    "msg_group_invites",
    "msg_dm_invites",
    "msg_calls",
];

/// Whether the store holds what a person sees as the data of an account
/// ([`SEEN_TABLES`]): a store whose owner nothing tells is wiped for a new
/// key only then. A fresh install keeps what was set before its first key
/// (the servers, the cursors a restore starts from).
pub async fn holds_data(store: &Store) -> Result<bool> {
    for table in SEEN_TABLES {
        let any: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT EXISTS (SELECT 1 FROM {table})")))
            .fetch_one(store.pool())
            .await
            .map_err(storage)?;
        if any != 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The data is of `pubkey_hex` from now on.
pub async fn set_owner(store: &Store, pubkey_hex: &str) -> Result<()> {
    settings::set(store, KEY_OWNER, pubkey_hex).await
}

/// Empty what the account kept, in one transaction: the tables of
/// [`ACCOUNT_TABLES`], the settings of [`ACCOUNT_SETTINGS`] and the record
/// of the owner. The keys and the link secrets of the groups are in the
/// secret store: their names join `groups.secrets_owed`, which the group service takes out
/// (`GroupService::forget_gone`). The files of the media and the avatars
/// are the runtime's to remove.
pub async fn wipe(store: &Store) -> Result<()> {
    let mut tx = store.pool().begin().await.map_err(storage)?;
    let keys: Vec<(String, String)> =
        sqlx::query_as("SELECT group_id, key_id FROM msg_group_keys ORDER BY group_id, key_id")
            .fetch_all(&mut *tx)
            .await
            .map_err(storage)?;
    let mut owed = crate::groups::secrets_owed_in(&mut tx).await?;
    for key in keys {
        if !owed.contains(&key) {
            owed.push(key);
        }
    }
    crate::groups::owe_links_in(&mut tx, None, &mut owed).await?;
    let existing: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table'")
        .fetch_all(&mut *tx)
        .await
        .map_err(storage)?;
    for table in ACCOUNT_TABLES.iter().filter(|t| existing.iter().any(|e| e == *t)) {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table}")))
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    for area in ACCOUNT_SETTINGS {
        // An area is a word of letters and `_`; `_` is LIKE's one-character
        // wildcard, so the area is compared as it is instead.
        sqlx::query("DELETE FROM msg_settings WHERE substr(key, 1, length(?1) + 1) = ?1 || '.'")
            .bind(area)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
    }
    sqlx::query("DELETE FROM msg_settings WHERE key = ?")
        .bind(KEY_OWNER)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    crate::groups::set_secrets_owed_in(&mut tx, &owed).await?;
    tx.commit().await.map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_empty_store_holds_no_data_of_an_account() {
        let store = Store::open_in_memory().await.unwrap();
        settings::set(&store, "privacy.read_receipts", "0").await.unwrap();
        crate::cursors::advance(&store, "dm.inbox", "", 5).await.unwrap();
        assert!(!holds_data(&store).await.unwrap(), "settings and cursors are no data a person sees");
        for table in SEEN_TABLES {
            assert!(ACCOUNT_TABLES.contains(&table), "{table}");
        }
        crate::chats::ensure_dm(&store, "bb", None).await.unwrap();
        assert!(holds_data(&store).await.unwrap());
    }

    async fn count(store: &Store, table: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
            .fetch_one(store.pool())
            .await
            .unwrap()
    }

    /// Every table of the messenger is the device's or the account's: a
    /// table a later migration adds is put in one list, so that a wipe
    /// never leaves the account's rows behind by not knowing them.
    #[tokio::test]
    async fn every_table_is_the_devices_or_the_accounts() {
        let store = Store::open_in_memory().await.unwrap();
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'",
        )
        .fetch_all(store.pool())
        .await
        .unwrap();
        for table in &tables {
            assert!(
                KEPT_TABLES.contains(&table.as_str()) || ACCOUNT_TABLES.contains(&table.as_str()),
                "the table {table} is in neither list of account.rs"
            );
        }
        for table in KEPT_TABLES {
            assert!(!ACCOUNT_TABLES.contains(&table), "{table} is in both lists");
        }
    }

    #[tokio::test]
    async fn the_owner_is_the_record_the_identity_or_the_author_of_a_sent_message() {
        let store = Store::open_in_memory().await.unwrap();
        assert_eq!(owner(&store).await.unwrap(), None);

        sqlx::query(
            "INSERT INTO msg_messages (id, chat_id, direction, status, content_type, envelope_json, sender_pubkey, created_at, received_at)
             VALUES ('m1', 'dm:bb', 'in', 'received', 'text', '{}', 'bb', 1, 1),
                    ('m2', 'dm:bb', 'out', 'sent', 'text', '{}', 'aa', 2, 2)",
        )
        .execute(store.pool())
        .await
        .unwrap();
        assert_eq!(owner(&store).await.unwrap().as_deref(), Some("aa"));

        identity::insert(
            &store,
            &identity::IdentityRow {
                id: "primary".into(),
                npub: "npub1cc".into(),
                pubkey_hex: "cc".into(),
                nsec_ref: "identity.nsec".into(),
                created_at: 1,
            },
        )
        .await
        .unwrap();
        assert_eq!(owner(&store).await.unwrap().as_deref(), Some("cc"));

        set_owner(&store, "dd").await.unwrap();
        assert_eq!(owner(&store).await.unwrap().as_deref(), Some("dd"));
    }

    #[tokio::test]
    async fn a_wipe_empties_the_account_and_keeps_the_device() {
        let store = Store::open_in_memory().await.unwrap();
        crate::chats::ensure_dm(&store, "bb", None).await.unwrap();
        sqlx::query(
            "INSERT INTO msg_messages (id, chat_id, direction, status, content_type, envelope_json, sender_pubkey, created_at, received_at)
             VALUES ('m1', 'dm:bb', 'out', 'sent', 'text', '{}', 'aa', 1, 1)",
        )
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query("INSERT INTO msg_group_keys (group_id, key_id, created_at) VALUES ('g1', 'k1', 1)")
            .execute(store.pool())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO msg_groups (id, kind, name, relay_url, owner, membership, link_epoch, created_at, updated_at)
             VALUES ('g1', 'public', 'G', 'wss://r.example', 'aa', 'joined', 2, 1, 1)",
        )
        .execute(store.pool())
        .await
        .unwrap();
        for (key, value) in [
            ("group.g1.oldest_fetched", "5"),
            ("groups.self_welcome.g1", "x"),
            ("privacy.read_receipts", "0"),
            ("relations.rev", "3"),
            ("ui_prefs.map", "{}"),
            ("dm.relays_published", "home"),
            ("servers.mode", "veydan"),
            ("net.mode", "auto"),
            ("notify.content", "full"),
            ("group_callx.kept", "1"),
        ] {
            settings::set(&store, key, value).await.unwrap();
        }
        set_owner(&store, "aa").await.unwrap();

        wipe(&store).await.unwrap();

        assert_eq!(count(&store, "msg_chats").await, 0);
        assert_eq!(count(&store, "msg_messages").await, 0);
        assert_eq!(count(&store, "msg_group_keys").await, 0);
        assert_eq!(owner(&store).await.unwrap(), None);
        for gone in ["group.g1.oldest_fetched", "groups.self_welcome.g1", "privacy.read_receipts", "relations.rev", "ui_prefs.map", "dm.relays_published"] {
            assert_eq!(settings::get(&store, gone).await.unwrap(), None, "{gone}");
        }
        for kept in ["servers.mode", "net.mode", "notify.content", "group_callx.kept"] {
            assert!(settings::get(&store, kept).await.unwrap().is_some(), "{kept}");
        }
        // The key and the links of every epoch of the wiped group are owed
        // to the secret store (ROO-681: the box outlives the store).
        assert_eq!(count(&store, "msg_groups").await, 0);
        let owed: Vec<(String, String)> = ["k1", "link.0", "link.1", "link.2"].iter().map(|k| ("g1".to_string(), k.to_string())).collect();
        assert_eq!(crate::groups::secrets_owed(&store).await.unwrap(), owed);
    }
}

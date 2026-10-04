-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Stage 5a: chats and messages. A DM chat id is `dm:<peer hex>`; a
-- message id is the rumor id (identical on every copy: peer's wrap,
-- self-copy, other device) so duplicates collapse on the primary key.
-- `msg_dm_routes` caches where a peer wants DMs delivered (kind 10050,
-- falling back to kind 10002).

CREATE TABLE IF NOT EXISTS msg_chats (
    id              TEXT PRIMARY KEY NOT NULL,
    kind            TEXT NOT NULL,                 -- dm (groups add their own)
    peer_pubkey     TEXT,
    unread          INTEGER NOT NULL DEFAULT 0,
    last_message_at INTEGER,
    last_preview    TEXT,
    pinned          INTEGER NOT NULL DEFAULT 0,
    archived        INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS msg_chats_order ON msg_chats(archived, pinned, last_message_at);

CREATE TABLE IF NOT EXISTS msg_messages (
    id              TEXT PRIMARY KEY NOT NULL,     -- rumor id, or sys:<random> for local system rows
    chat_id         TEXT NOT NULL,
    wire_id         TEXT,                          -- outer wrap id seen or sent first
    direction       TEXT NOT NULL,                 -- in | out
    status          TEXT NOT NULL,                 -- queued | sent | failed | received
    content_type    TEXT NOT NULL,                 -- text | edit | delete | control | system | media
    text            TEXT,                          -- current display text (edits rewrite it)
    envelope_json   TEXT NOT NULL,
    sender_pubkey   TEXT NOT NULL,
    reply_to_id     TEXT,
    target_id       TEXT,                          -- edit/delete: the message they act on
    created_at      INTEGER NOT NULL,              -- rumor created_at (application time)
    received_at     INTEGER NOT NULL,
    edited_at       INTEGER,
    deleted_at      INTEGER,
    is_hidden       INTEGER NOT NULL DEFAULT 0,    -- edit/delete/control rows never render
    outbox_local_id TEXT,
    failure_reason  TEXT,
    media_json      TEXT
);
CREATE INDEX IF NOT EXISTS msg_messages_chat_time ON msg_messages(chat_id, is_hidden, created_at);
CREATE INDEX IF NOT EXISTS msg_messages_outbox ON msg_messages(outbox_local_id);
CREATE INDEX IF NOT EXISTS msg_messages_target ON msg_messages(target_id);

CREATE TABLE IF NOT EXISTS msg_dm_routes (
    peer_pubkey      TEXT NOT NULL,
    relay_url        TEXT NOT NULL,
    source           TEXT NOT NULL,                -- nip17 | nip65
    event_created_at INTEGER NOT NULL DEFAULT 0,
    updated_at       INTEGER NOT NULL,
    PRIMARY KEY (peer_pubkey, relay_url)
);

-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Stage 8: groups. The truth about a group is its operation log
-- (`msg_group_ops`); `msg_groups` caches what lists need. Key bytes and
-- link secrets are not here: they live in the SecretStore under
-- `group.<id>.key.<key_id>` and `group.<id>.link`.
--
-- The chat of a group is the `msg_chats` row `group:<id>`; its messages
-- are ordinary `msg_messages` rows.

CREATE TABLE IF NOT EXISTS msg_groups (
    id         TEXT PRIMARY KEY NOT NULL,     -- 64 hex
    kind       TEXT NOT NULL,                 -- public | private
    name       TEXT NOT NULL,
    about      TEXT NOT NULL DEFAULT '',
    picture    TEXT NOT NULL DEFAULT '',
    relay_url  TEXT NOT NULL,
    owner      TEXT NOT NULL,
    -- joined | left | removed | banned | disbanded
    membership TEXT NOT NULL,
    my_role    TEXT,
    members    INTEGER NOT NULL DEFAULT 0,
    link_epoch INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS msg_group_ops (
    op_id       TEXT PRIMARY KEY NOT NULL,
    group_id    TEXT NOT NULL,
    op_json     TEXT NOT NULL,
    received_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS msg_group_ops_group ON msg_group_ops(group_id);

CREATE TABLE IF NOT EXISTS msg_group_keys (
    group_id   TEXT NOT NULL,
    key_id     TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (group_id, key_id)
);

-- Events that arrived before the key that opens them.
CREATE TABLE IF NOT EXISTS msg_group_pending (
    event_id    TEXT PRIMARY KEY NOT NULL,
    group_id    TEXT NOT NULL,
    key_id      TEXT NOT NULL,
    ciphertext  TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    received_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS msg_group_pending_key ON msg_group_pending(group_id, key_id);

CREATE TABLE IF NOT EXISTS msg_group_invites (
    invite_id    TEXT PRIMARY KEY NOT NULL,
    group_id     TEXT NOT NULL,
    direction    TEXT NOT NULL,               -- in | out
    peer         TEXT NOT NULL,               -- inviter (in) or invitee (out)
    -- sent | received | accepted | declined | done | expired | cancelled
    status       TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS msg_group_invites_group ON msg_group_invites(group_id, status);

-- Requests to join a private group. Incoming ones sit with the managers;
-- the outgoing one is the proof that a welcome was asked for.
CREATE TABLE IF NOT EXISTS msg_group_requests (
    group_id     TEXT NOT NULL,
    requester    TEXT NOT NULL,
    direction    TEXT NOT NULL,               -- in | out
    status       TEXT NOT NULL,               -- pending | approved | rejected
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (group_id, requester, direction)
);

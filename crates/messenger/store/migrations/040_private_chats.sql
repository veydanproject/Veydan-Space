-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Private chats (internal/messenger-wire.md, «Частный чат»). One person
-- may be talked to twice: in the chat of the home servers (`dm:<peer>`)
-- and in a chat that lives on one private server only
-- (`dm:<peer>@<server_id>`). Each is its own conversation, with its own
-- relationship: blocked at home and open on the server is a state the
-- user may choose.
--
--   msg_chats.server_id        the private server of a private chat; NULL
--                              for every chat of the home set, which is
--                              every row there was before
--   msg_dm_relations.server_id the server of the relation, '' for the
--                              home chat; part of the primary key, so the
--                              relation of a private chat never touches
--                              the home one. Every row of before is the
--                              home relation.
--   msg_dm_routes.server_id    the server whose relay told the peer's
--                              inbox list, '' for the home set: a list a
--                              peer publishes on a private server names
--                              that server's relays, and must not replace
--                              where home messages are delivered.
--
-- SQLite cannot change a primary key in place, so both tables are made
-- anew with their rows. `msg_presence_keys` and `msg_presence_told` stay
-- keyed by the peer: presence is a person's (one heartbeat key each,
-- beaten on the home set), not a chat's.

ALTER TABLE msg_chats ADD COLUMN server_id TEXT;

CREATE TABLE msg_dm_relations_new (
    peer_pubkey         TEXT NOT NULL,
    server_id           TEXT NOT NULL DEFAULT '',
    my_contact          TEXT NOT NULL DEFAULT 'none',
    blocked             INTEGER NOT NULL DEFAULT 0,
    peer_signal         TEXT NOT NULL DEFAULT 'none',
    was_ever_mutual     INTEGER NOT NULL DEFAULT 0,
    last_signal_at      INTEGER NOT NULL DEFAULT 0,
    last_my_signal_at   INTEGER NOT NULL DEFAULT 0,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    request_floor       INTEGER NOT NULL DEFAULT 0,
    request_end_pending INTEGER NOT NULL DEFAULT 0,
    decided_at          INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (peer_pubkey, server_id)
);

INSERT INTO msg_dm_relations_new (peer_pubkey, server_id, my_contact, blocked, peer_signal, was_ever_mutual,
                                  last_signal_at, last_my_signal_at, created_at, updated_at, request_floor,
                                  request_end_pending, decided_at)
SELECT peer_pubkey, '', my_contact, blocked, peer_signal, was_ever_mutual, last_signal_at, last_my_signal_at,
       created_at, updated_at, request_floor, request_end_pending, decided_at
FROM msg_dm_relations;

DROP TABLE msg_dm_relations;
ALTER TABLE msg_dm_relations_new RENAME TO msg_dm_relations;

CREATE TABLE msg_dm_routes_new (
    peer_pubkey      TEXT NOT NULL,
    server_id        TEXT NOT NULL DEFAULT '',
    relay_url        TEXT NOT NULL,
    source           TEXT NOT NULL,
    event_created_at INTEGER NOT NULL DEFAULT 0,
    updated_at       INTEGER NOT NULL,
    PRIMARY KEY (peer_pubkey, server_id, relay_url)
);

INSERT INTO msg_dm_routes_new (peer_pubkey, server_id, relay_url, source, event_created_at, updated_at)
SELECT peer_pubkey, '', relay_url, source, event_created_at, updated_at FROM msg_dm_routes;

DROP TABLE msg_dm_routes;
ALTER TABLE msg_dm_routes_new RENAME TO msg_dm_routes;

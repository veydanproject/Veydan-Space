-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Servers (internal/messenger-wire.md, «Серверы»). A server is one relay
-- with its app key, and what goes with it: a blob server, a call node, a
-- push mode, a name. Its id is the first 16 hex of the SHA-256 of the
-- relay url (lowercase, no trailing slash), so every device of mine names
-- the same server the same way without telling the others.
--
--   kind  home     the relays the messenger uses as before: the manifest's
--                  in Veydan mode, the user's own in own mode; one row per
--                  relay url (`RelayService::reload` keeps them)
--         private  added from a server card: nothing is sent there and
--                  nothing of it is listened to unless a scope names it
--
-- `msg_relays.server_id` names the server a relay row belongs to; NULL is
-- the home set, which is every row there was before. The same for
-- `msg_media_servers.server_id`: a blob server of a private server is
-- written to only for that server. Its `access_key`, unused by blossom
-- rows until now, holds the server's app key there, sent as `?key=` on
-- the upload.
--
-- `msg_sync_cursors.relay_url` holds the server id of a private server's
-- cursor; `*` (`cursors::ANY_RELAY`) stays the home set's, so the rows
-- already there keep meaning what they meant.

CREATE TABLE IF NOT EXISTS msg_servers (
    id         TEXT PRIMARY KEY NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('home', 'private')),
    name       TEXT,
    relay_url  TEXT NOT NULL UNIQUE,
    relay_key  TEXT,
    media_url  TEXT,
    call_node  TEXT,
    push_mode  TEXT NOT NULL DEFAULT 'off' CHECK (push_mode IN ('off', 'veydan')),
    source     TEXT,
    added_at   INTEGER NOT NULL
);

ALTER TABLE msg_relays ADD COLUMN server_id TEXT;
ALTER TABLE msg_media_servers ADD COLUMN server_id TEXT;

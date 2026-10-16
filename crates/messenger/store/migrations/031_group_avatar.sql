-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- The pictures of the groups I manage, as this device looks after them:
-- one row per group whose log names a picture this device holds the
-- file of (`avatars/group/<sha256>.jpg`). Like `msg_own_avatar`: the hash
-- of the JPEG, the address the log names, the server it is on (when it
-- is one of mine), where it was put (`copies_json`, when this device put
-- it), when it was set, when its presence on the server was last checked
-- and when it was last fetched to keep it alive.

CREATE TABLE IF NOT EXISTS msg_group_avatar (
    group_id    TEXT PRIMARY KEY NOT NULL,
    sha256      TEXT NOT NULL,
    url         TEXT NOT NULL,
    server_id   TEXT,
    copies_json TEXT NOT NULL DEFAULT '[]',
    set_at      INTEGER NOT NULL,
    checked_at  INTEGER NOT NULL DEFAULT 0,
    touched_at  INTEGER NOT NULL DEFAULT 0
);

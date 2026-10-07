-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Profile, avatar and contact card.
--
-- `msg_profiles.about_rich`: the bio with its marks (`veydan_about` of the
-- kind 0), kept only when its text without the marks is the `about` other
-- clients see; otherwise NULL and the bio is the plain `about`.
-- `msg_profiles.socials_json`: the checked links of `veydan_socials`, as a
-- JSON list of `{"p","h"}`; NULL when there are none.
--
-- `msg_own_private`: what of my profile never goes into kind 0: my phone
-- and whether my own card carries it by default. One row; my devices agree
-- on it by own notes, the later `updated_at` wins.
--
-- `msg_own_avatar`: my avatar as uploaded: the hash of the JPEG, its
-- address in kind 0, the server it was put on first and every copy
-- (`copies_json`), when it was set, when its presence on the servers was
-- last checked and when it was last fetched to keep it alive.
--
-- `msg_avatar_cache`: the avatars of others by their address: the hash
-- the cached file is named by, when it was fetched, and the failures
-- since the last success.
--
-- `msg_contact_private`: what I know of a contact privately, from the card
-- it sent me: its phone. My devices agree on it by own notes, the later
-- `updated_at` wins per contact.

ALTER TABLE msg_profiles ADD COLUMN about_rich TEXT;
ALTER TABLE msg_profiles ADD COLUMN socials_json TEXT;

CREATE TABLE IF NOT EXISTS msg_own_private (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    phone       TEXT,
    share_phone INTEGER NOT NULL DEFAULT 0,
    updated_at  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS msg_own_avatar (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    sha256      TEXT NOT NULL,
    url         TEXT NOT NULL,
    server_id   TEXT,
    copies_json TEXT NOT NULL DEFAULT '[]',
    set_at      INTEGER NOT NULL,
    checked_at  INTEGER NOT NULL DEFAULT 0,
    touched_at  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS msg_avatar_cache (
    url        TEXT PRIMARY KEY NOT NULL,
    sha256     TEXT,
    fetched_at INTEGER NOT NULL DEFAULT 0,
    failed_at  INTEGER NOT NULL DEFAULT 0,
    attempts   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS msg_contact_private (
    pubkey     TEXT PRIMARY KEY NOT NULL,
    phone      TEXT,
    updated_at INTEGER NOT NULL
);

-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Stage 4: profiles (kind-0 cache, including our own), the private address
-- book, and our public follow list (kind 3).

CREATE TABLE IF NOT EXISTS msg_profiles (
    pubkey           TEXT PRIMARY KEY NOT NULL,
    name             TEXT,
    display_name     TEXT,
    about            TEXT,
    picture          TEXT,
    banner           TEXT,
    website          TEXT,
    nip05            TEXT,
    lud16            TEXT,
    nip05_verified_at INTEGER,
    -- created_at of the kind-0 event this row reflects (LWW key).
    event_created_at INTEGER NOT NULL DEFAULT 0,
    fetched_at       INTEGER NOT NULL,
    raw_json         TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS msg_private_contacts (
    pubkey             TEXT PRIMARY KEY NOT NULL,
    nickname           TEXT,
    note               TEXT,
    is_muted           INTEGER NOT NULL DEFAULT 0,
    notification_level TEXT NOT NULL DEFAULT 'all',   -- all | mentions | none
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    deleted_at         INTEGER
);
CREATE INDEX IF NOT EXISTS msg_private_contacts_active ON msg_private_contacts(deleted_at, updated_at);

CREATE TABLE IF NOT EXISTS msg_follows (
    pubkey   TEXT PRIMARY KEY NOT NULL,
    added_at INTEGER NOT NULL
);

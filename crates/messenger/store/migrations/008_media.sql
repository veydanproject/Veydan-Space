-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Stage 6: blob servers and transfers.
--
-- `msg_media_servers`: where we upload. `kind` is s3 (default choice) or
-- blossom. The S3 secret key is not here: it lives in the SecretStore
-- under `media.<id>.secret`.
--
-- `msg_transfers`: one row per upload or download. `state_json` holds what
-- a resume needs (for uploads: key, nonce and the chunks already stored;
-- for downloads: the descriptor). `attempts = -1` on a download means the
-- user cancelled it: never start it again automatically.

CREATE TABLE IF NOT EXISTS msg_media_servers (
    id         TEXT PRIMARY KEY NOT NULL,
    kind       TEXT NOT NULL,                 -- s3 | blossom
    url        TEXT NOT NULL,                 -- s3 endpoint or blossom base
    bucket     TEXT,
    region     TEXT,
    access_key TEXT,
    priority   INTEGER NOT NULL DEFAULT 100,  -- lower first
    enabled    INTEGER NOT NULL DEFAULT 1,
    source     TEXT NOT NULL DEFAULT 'user',  -- manifest | user
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS msg_transfers (
    id             TEXT PRIMARY KEY NOT NULL,
    direction      TEXT NOT NULL,             -- up | down
    message_id     TEXT,
    chat_id        TEXT,
    local_path     TEXT,
    file_name      TEXT NOT NULL,
    mime           TEXT NOT NULL,
    size           INTEGER NOT NULL,
    sha256         TEXT,
    status         TEXT NOT NULL,             -- queued | running | paused | done | failed | cancelled
    done_bytes     INTEGER NOT NULL DEFAULT 0,
    attempts       INTEGER NOT NULL DEFAULT 0,
    failure_reason TEXT,
    state_json     TEXT NOT NULL DEFAULT '{}',
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS msg_transfers_message ON msg_transfers(message_id);
CREATE INDEX IF NOT EXISTS msg_transfers_status ON msg_transfers(status);

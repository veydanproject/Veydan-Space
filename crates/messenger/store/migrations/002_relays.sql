-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Stage 2: relay pool configuration. Manifest-managed rows carry the stable
-- `relay_id`; user rows have NULL. Manifest bookkeeping (serial, region,
-- silent mode) lives in msg_settings under `manifest.*` / `relays.*`.

CREATE TABLE IF NOT EXISTS msg_relays (
    url           TEXT PRIMARY KEY NOT NULL,
    relay_id      TEXT,
    source        TEXT NOT NULL,              -- 'manifest' | 'user'
    regions_json  TEXT NOT NULL DEFAULT '[]',
    read          INTEGER NOT NULL DEFAULT 1,
    write         INTEGER NOT NULL DEFAULT 1,
    enabled       INTEGER NOT NULL DEFAULT 1,
    auth_type     TEXT,                       -- NULL | 'nip42'
    failures      INTEGER NOT NULL DEFAULT 0,
    last_ok_at    INTEGER,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS msg_relays_relay_id ON msg_relays(relay_id) WHERE relay_id IS NOT NULL;

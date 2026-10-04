-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Stage 0/1: identity and module settings. All times are unix seconds.

CREATE TABLE IF NOT EXISTS msg_settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

-- One row per local identity. `nsec_ref` is the SecretStore key under which
-- the private key lives; the key material itself never enters this database.
CREATE TABLE IF NOT EXISTS msg_identity (
    id         TEXT PRIMARY KEY NOT NULL,
    npub       TEXT NOT NULL UNIQUE,
    pubkey_hex TEXT NOT NULL UNIQUE,
    nsec_ref   TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

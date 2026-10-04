-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Stage 3: ingress bookkeeping. `msg_events_raw` is the dedup set of wire
-- events (and the local id set for reconciliation); `msg_outbox` persists
-- outgoing requests until a relay accepts them; `msg_sync_cursors` records
-- how far each scope has been synced per relay.

CREATE TABLE IF NOT EXISTS msg_events_raw (
    event_id    TEXT PRIMARY KEY NOT NULL,
    kind        INTEGER NOT NULL,
    pubkey      TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    received_at INTEGER NOT NULL,
    source_json TEXT NOT NULL,
    raw_json    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS msg_events_raw_kind_time ON msg_events_raw(kind, created_at);

CREATE TABLE IF NOT EXISTS msg_outbox (
    local_id      TEXT PRIMARY KEY NOT NULL,
    outbound_json TEXT NOT NULL,
    state         TEXT NOT NULL,             -- queued | publishing | published | failed
    attempts      INTEGER NOT NULL DEFAULT 0,
    next_retry_at INTEGER NOT NULL,
    last_error    TEXT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS msg_outbox_due ON msg_outbox(state, next_retry_at);

CREATE TABLE IF NOT EXISTS msg_sync_cursors (
    scope         TEXT NOT NULL,
    relay_url     TEXT NOT NULL,
    last_event_at INTEGER NOT NULL,
    PRIMARY KEY (scope, relay_url)
);

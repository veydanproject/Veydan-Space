-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- The archive of the conversation in the user's sync storage
-- (internal/messenger-archive-plan.md §4, internal/messenger-wire.md §8,
-- crate messenger-archive).
--
-- msg_archive_events: every event of the archive, by segment (scope ×
-- month). The archive's own copy: a personal message is kept opened (the
-- rumor), which `msg_events_raw` cannot give back without the nsec, and a
-- segment file is rebuilt from here.
--
--   id          the id of the wire event (the wrap of a personal message)
--   scope       `dm`, `dm@<server>`, `group:<id>`, `meta`
--   month       `yyyy-mm` of `at`, UTC
--   at          the application time (the rumor's of a personal message):
--               the sender's clock, so the high-water mark of a scope caps
--               it at received_at + skew (messenger-archive AT_SKEW_SECS)
--   received_at when a device received it (the writer's, for one merged)
--   kind        the wire kind
--   msg_id      the rumor id of a personal message (`msg_messages.id`),
--               for compaction; NULL for a wire body
--   seq         the place of the event in the segment file, fixed the
--               first time a file holding it is built or read: a rewrite
--               keeps the order, so a grown month changes its last chunk
--               alone. NULL until then; the first remote_count places are
--               the storage's file.
--   replayed    0 while an event merged from the storage has not gone
--               through the ingress yet (a restart resumes it)
--   dropped     1 once compaction took it out: the row stays, without
--               its body, so a merge does not bring it back
--   event_json  the record as the file holds it (`ArchivedEvent`), '' when
--               dropped
CREATE TABLE IF NOT EXISTS msg_archive_events (
    id          TEXT PRIMARY KEY NOT NULL,
    scope       TEXT NOT NULL,
    month       TEXT NOT NULL,
    at          INTEGER NOT NULL,
    received_at INTEGER NOT NULL,
    kind        INTEGER NOT NULL,
    msg_id      TEXT,
    seq         INTEGER,
    replayed    INTEGER NOT NULL DEFAULT 1,
    dropped     INTEGER NOT NULL DEFAULT 0,
    event_json  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS msg_archive_events_segment ON msg_archive_events (scope, month, at);
-- The high-water mark of a scope, MAX(MIN(at, received_at + skew)) over the
-- scope, is read from this index alone.
CREATE INDEX IF NOT EXISTS msg_archive_events_mark ON msg_archive_events (scope, at, received_at);
CREATE INDEX IF NOT EXISTS msg_archive_events_msg ON msg_archive_events (msg_id) WHERE msg_id IS NOT NULL;

-- msg_archive_state: one row per segment this device holds events of.
--
--   dirty         1 when the device holds events the storage may lack
--   local_count   live (not dropped) events the device holds
--   written_*     the file the device last wrote, or found equal to its
--                 own (count, sha256 hex, when)
--   remote_*      the file of the storage the device last read
CREATE TABLE IF NOT EXISTS msg_archive_state (
    scope         TEXT NOT NULL,
    month         TEXT NOT NULL,
    dirty         INTEGER NOT NULL DEFAULT 0,
    local_count   INTEGER NOT NULL DEFAULT 0,
    written_count INTEGER,
    written_hash  TEXT,
    written_at    INTEGER,
    remote_count  INTEGER,
    remote_hash   TEXT,
    PRIMARY KEY (scope, month)
);
CREATE INDEX IF NOT EXISTS msg_archive_state_dirty ON msg_archive_state (dirty) WHERE dirty = 1;

-- msg_archive_queue: segments the storage holds that this device is still
-- to replay (the restore of a fresh device, a month another device grew).
--
--   entity_id  `<owner>/<scope>/<yyyy-mm>`, the id of the sync entity
--   count, size, hash   of the version announced by the op
--   status     pending | done | failed | parked | refused
--              failed: the storage did not give it, tried again next round;
--              parked: the runtime refuses it for now (a group or a server
--              this device has no row for), tried again when one comes;
--              refused: a file this build does not read, tried again by
--              another build
--   build      the build that refused it, '' otherwise
--   priority   the higher the sooner (newest months first, `meta` above all)
CREATE TABLE IF NOT EXISTS msg_archive_queue (
    entity_id  TEXT PRIMARY KEY NOT NULL,
    scope      TEXT NOT NULL,
    month      TEXT NOT NULL,
    count      INTEGER NOT NULL,
    size       INTEGER NOT NULL,
    hash       TEXT NOT NULL,
    status     TEXT NOT NULL DEFAULT 'pending',
    priority   INTEGER NOT NULL,
    error      TEXT,
    build      TEXT NOT NULL DEFAULT '',
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS msg_archive_queue_next ON msg_archive_queue (status, priority);

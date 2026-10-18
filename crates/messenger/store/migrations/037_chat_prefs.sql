-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- The choices of a chat that every device of mine shares: no
-- notifications, pinned, archived (internal/messenger-wire.md §3, «Снимок
-- отношений», `chats`). Each with the time it was made (my clock, unix
-- seconds; 0 = never chosen on any device this one heard of): the newer
-- choice wins between devices. A row may name a chat this device does not
-- have yet (a new device reads the snapshot before the history): the chat
-- takes it when it is made (`chats::take_prefs`). `msg_chats` keeps the
-- flags the list shows; this is what was chosen.

CREATE TABLE IF NOT EXISTS msg_chat_prefs (
    chat_id      TEXT PRIMARY KEY NOT NULL,
    muted        INTEGER NOT NULL DEFAULT 0,
    muted_at     INTEGER NOT NULL DEFAULT 0,
    pinned       INTEGER NOT NULL DEFAULT 0,
    pinned_at    INTEGER NOT NULL DEFAULT 0,
    archived     INTEGER NOT NULL DEFAULT 0,
    archived_at  INTEGER NOT NULL DEFAULT 0
);

-- What this device chose before the choices were shared, dated now: each
-- device that updates hands its own to the others, and nothing is undone
-- (a flag that is off was never a choice).
INSERT INTO msg_chat_prefs (chat_id, muted, muted_at, pinned, pinned_at, archived, archived_at)
SELECT id,
       muted,    CASE WHEN muted    = 1 THEN CAST(strftime('%s', 'now') AS INTEGER) ELSE 0 END,
       pinned,   CASE WHEN pinned   = 1 THEN CAST(strftime('%s', 'now') AS INTEGER) ELSE 0 END,
       archived, CASE WHEN archived = 1 THEN CAST(strftime('%s', 'now') AS INTEGER) ELSE 0 END
  FROM msg_chats
 WHERE muted = 1 OR pinned = 1 OR archived = 1;

-- The snapshot is owed, so the next start publishes them.
INSERT OR REPLACE INTO msg_settings (key, value)
SELECT 'relations.owed', '1' WHERE EXISTS (SELECT 1 FROM msg_chat_prefs);

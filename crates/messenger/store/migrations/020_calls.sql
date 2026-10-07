-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Calls (messenger-calls). One row per call, by its id: the invitation,
-- the answer and the end all name it, from whichever device of mine or of
-- the peer they come, so the copies of several devices meet in one row.
--
-- `outcome` is NULL while the call is under way, then one of `missed`
-- (nobody took it), `declined`, `busy`, `ended` (it was talked) and
-- `failed` (no connection). `via_relay` says whether the media went
-- through a TURN relay rather than directly.
--
-- The line in the chat is a system row of `msg_messages`
-- (`sys:call:<call_id>`, content type `system`, text `call`), with the
-- same facts in `media_json`; this table is the record, the row is what
-- the chat shows.

CREATE TABLE IF NOT EXISTS msg_calls (
    call_id TEXT PRIMARY KEY NOT NULL,
    chat_id TEXT NOT NULL,
    peer TEXT NOT NULL,
    direction TEXT NOT NULL,
    media TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    answered_at INTEGER,
    ended_at INTEGER,
    outcome TEXT,
    via_relay INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS msg_calls_chat ON msg_calls (chat_id, started_at);

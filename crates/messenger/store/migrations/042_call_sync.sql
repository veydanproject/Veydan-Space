-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- The journal of calls the same on every device of mine
-- (internal/messenger-wire.md §3, `own.call`, `own.call_hide`).
--
-- What came of a call before its record did: the relays keep no order, a
-- catch-up gives the newest first, and an invitation outlives no minute
-- on them while the answer and the end live five. The answer, the end and
-- a hiding of my other device wait here until the record is made (by the
-- invitation, or by the note of my other device), and are written into it
-- then; a restart of the app between the two loses nothing. `noted_at` is
-- when this device heard the first word: a word of a call that never
-- comes goes after a week.

CREATE TABLE IF NOT EXISTS msg_call_early (
    call_id TEXT PRIMARY KEY NOT NULL,
    answered_at INTEGER,
    outcome TEXT,
    ended_at INTEGER,
    hidden_at INTEGER,
    noted_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS msg_call_early_noted ON msg_call_early (noted_at);

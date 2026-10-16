-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- The journal of calls (the section «Звонки»): every call of `msg_calls`
-- across the chats, newest first. A call taken out of the journal keeps
-- its row, with `hidden_at` set: a late word of the call (an end from
-- another device) still finds it, and its line in the chat stays.

ALTER TABLE msg_calls ADD COLUMN hidden_at INTEGER;
CREATE INDEX IF NOT EXISTS msg_calls_started ON msg_calls (started_at);

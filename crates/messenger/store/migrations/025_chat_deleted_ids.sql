-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- What a deleted chat held, by rumor id. The time tombstone of 023 now
-- covers only my own copies: a message of the peer this device never had
-- is new whatever its clock says, so the peer's rows that were deleted are
-- remembered one by one and only those are dropped when they come again.
-- (Not `msg_own_hidden`: a message removed for me comes back as a deleted
-- line, one of a deleted chat does not come back at all.)
--
--   message_id  rumor id of a row the chat held when it was deleted
--   chat_id     the chat it was in
--   deleted_at  my clock at the deletion

CREATE TABLE IF NOT EXISTS msg_chat_deleted_ids (
    message_id TEXT PRIMARY KEY NOT NULL,
    chat_id    TEXT NOT NULL,
    deleted_at INTEGER NOT NULL
);

-- cleared_at is never further ahead of the clock than 300 s: one row dated
-- in the future must not hold back everything written before that date.
UPDATE msg_chat_cleared
SET cleared_at = CAST(strftime('%s', 'now') AS INTEGER) + 300
WHERE cleared_at > CAST(strftime('%s', 'now') AS INTEGER) + 300;

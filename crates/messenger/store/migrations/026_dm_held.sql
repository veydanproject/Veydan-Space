-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Held messages of the DM gate. A text of the peer that came as a second
-- message before approval was dropped for good; but NIP-59 wraps come in
-- any order, and the signal that ends the episode before it (a removal, a
-- withdrawal, a no) may come later. Such a message is now stored hidden
-- and marked here; when the request floor is next raised and nothing of
-- the peer is shown above it, the oldest held row above it is shown as the
-- new request. My approval, my block and deleting the chat purge them.
--
--   message_id  rumor id of the hidden row in msg_messages
--   chat_id     its chat
--   created_at  its rumor time (the peer's clock)

CREATE TABLE IF NOT EXISTS msg_dm_held (
    message_id TEXT PRIMARY KEY NOT NULL,
    chat_id    TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS msg_dm_held_chat ON msg_dm_held (chat_id, created_at);

-- An episode the peer ended while none of its messages was stored here
-- yet: the time of that signal (the peer's clock), 0 when none waits. The
-- first message of the peer stored after it and dated before it belongs to
-- that episode and raises the floor to itself; it is used once.
ALTER TABLE msg_dm_relations ADD COLUMN request_end_pending INTEGER NOT NULL DEFAULT 0;

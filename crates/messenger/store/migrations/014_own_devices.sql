-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- What one of my devices did, the others repeat: a chat read there is read
-- here, a message removed for me there is removed here. Both are kept by
-- their own key, so they hold whatever the order things arrive in: a mark
-- may come before the chat or the message it names.
--
-- `msg_own_read`: the time of the newest message of the peer that was read
-- on any of my devices. A message not later than it never counts as unread.
--
-- `msg_own_hidden`: messages removed for me. One that arrives later is
-- removed as it comes.

CREATE TABLE IF NOT EXISTS msg_own_read (
    chat_id TEXT PRIMARY KEY NOT NULL,
    read_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS msg_own_hidden (
    message_id TEXT PRIMARY KEY NOT NULL,
    hidden_at  INTEGER NOT NULL
);

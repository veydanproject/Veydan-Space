-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- A chat deleted on this device stays deleted. Relays keep the ciphertext,
-- so history sync and copies from my other devices would bring its rows
-- back; what was written up to the deletion is dropped on the way in.
--
--   cleared_at  rumor time up to which (inclusive) the chat was deleted:
--               the newest message it held or the clock, whichever is
--               later. Never lowered.

CREATE TABLE IF NOT EXISTS msg_chat_cleared (
    chat_id    TEXT PRIMARY KEY NOT NULL,
    cleared_at INTEGER NOT NULL
);

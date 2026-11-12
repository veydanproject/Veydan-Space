-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- The unsent text of a conversation, kept on this device so that it
-- outlives the app (ROO-1112): the composer writes it as it is typed and
-- forgets it when the field is sent or emptied. Never sent anywhere, not
-- to my other devices either; wiped with the account.
--
--   chat        the composer's key: a chat id, `group:<id>`, `<chat>#<post>`
--   text        as typed, never empty
--   md          1 when the field is written in Markdown
--   updated_at  my clock at the last write
CREATE TABLE IF NOT EXISTS msg_drafts (
    chat       TEXT PRIMARY KEY NOT NULL,
    text       TEXT NOT NULL,
    md         INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);

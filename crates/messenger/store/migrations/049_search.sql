-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- The search of the conversation (ROO-1137): every word of a message I
-- hold, found by the start of a word, in any script and case, with or
-- without its marks («Ёлка» finds «елка», «jose» «José»). The words are
-- folded in Rust before they come here (`search::fold`, the rule of
-- `fold()` of the UI), the tokenizer only cuts them; a query is folded
-- the same way. Kept on this device, never sent; wiped with the account.
--
-- `msg_search` holds no text of its own (`content=''`), only the index of
-- the words; `contentless_delete` lets a row be taken out when a message
-- is edited, deleted or goes with its chat. `msg_messages.search_rowid` is
-- the rowid of the message's words there (NULL: none), given by the store
-- and kept in the row: `msg_messages` has a text key, and its own rowid is
-- not one a VACUUM leaves alone.
--
-- What a message holds is put in by `search::index_in` wherever its text,
-- its file or its row changes; whether it is shown (moderation, a block,
-- removed for me) is asked when searching, as the chat asks it. The rows
-- that came before this migration are put in by `search::backfill_batch`
-- in the background (`search.backfilled_to` in msg_settings).
CREATE VIRTUAL TABLE IF NOT EXISTS msg_search USING fts5(
    body,
    content = '',
    contentless_delete = 1,
    tokenize = 'unicode61 remove_diacritics 0'
);

ALTER TABLE msg_messages ADD COLUMN search_rowid INTEGER;
CREATE INDEX IF NOT EXISTS msg_messages_search ON msg_messages (search_rowid) WHERE search_rowid IS NOT NULL;

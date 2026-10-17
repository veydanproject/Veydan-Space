-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Messages written in Markdown (`"fmt":"md"` of a text, an edit or a
-- caption, internal/messenger-wire.md §3). `fmt` is how `text` is
-- written: `` plain, `md` Markdown; an edit sets both, so a message can
-- become Markdown or stop being it. `plain` is the text of a Markdown
-- message without its marks (messenger_richtext::md::plain), written with
-- `text` in Rust, and NULL for plain text: the list of chats, a quote and
-- a push show it, and `recompute_last` rebuilds the line of a chat from
-- it in SQL without parsing anything.

ALTER TABLE msg_messages ADD COLUMN fmt TEXT NOT NULL DEFAULT '';
ALTER TABLE msg_messages ADD COLUMN plain TEXT;

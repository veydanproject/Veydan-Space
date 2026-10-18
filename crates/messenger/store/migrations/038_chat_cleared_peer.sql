-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- A chat deleted on one device of mine is deleted on all of them
-- (internal/messenger-wire.md §3, «Снимок отношений», `deleted`). Here the
-- peer's rows were kept out by id alone (`msg_chat_deleted_ids`), and a
-- device that never had them has no ids: a new one would bring the whole
-- chat back from the relays. So the tombstone says up to when the peer's
-- rows go too:
--
--   peer_upto  the time of the newest message of the peer the chat held
--              when it was deleted (their clock; one dated past mine and
--              `CLEARED_SKEW_SECS` is left out). Never lowered. A later message of theirs is new, whatever their
--              clock says, as it was dated after one that was here.
--
-- A tombstone of before takes the clock of its deletion (`deleted_at` of
-- the rows it remembers), or its own `cleared_at` when it held none: the
-- deletion came after every message it held. Each is published at the
-- next start.

ALTER TABLE msg_chat_cleared ADD COLUMN peer_upto INTEGER NOT NULL DEFAULT 0;

UPDATE msg_chat_cleared
   SET peer_upto = MIN(cleared_at, COALESCE(
       (SELECT MAX(d.deleted_at) FROM msg_chat_deleted_ids d WHERE d.chat_id = msg_chat_cleared.chat_id), cleared_at));

INSERT OR REPLACE INTO msg_settings (key, value)
SELECT 'relations.owed', '1' WHERE EXISTS (SELECT 1 FROM msg_chat_cleared WHERE chat_id LIKE 'dm:%');

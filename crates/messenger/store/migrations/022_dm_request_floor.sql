-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- "One visible message before approval" counts per episode of a DM
-- relationship, not over the whole chat: a pair that talked before may
-- start again with a new request.
--
--   request_floor  rumor time, in the peer's clock, where the last episode
--                  ended; only incoming messages after it count toward the
--                  next request. Raised when either side ends an episode
--                  (remove, decline, withdraw, unblock of a stranger).

ALTER TABLE msg_dm_relations ADD COLUMN request_floor INTEGER NOT NULL DEFAULT 0;

-- Episodes that already ended (neither side approves now): what the peer
-- wrote so far belongs to them.
UPDATE msg_dm_relations SET request_floor = COALESCE((
    SELECT MAX(m.created_at) FROM msg_messages m
    WHERE m.chat_id = 'dm:' || msg_dm_relations.peer_pubkey
      AND m.direction = 'in' AND m.is_hidden = 0 AND m.content_type != 'system'
), 0)
WHERE my_contact != 'approved' AND peer_signal != 'approved';

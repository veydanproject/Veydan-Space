-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- DM requests as a queue with a term. When I decided about a peer (added,
-- accepted, declined, blocked, unblocked, or the request ran out), in my
-- clock, monotonic per peer. A copy of a decision from another device (a
-- signal's self-copy, later the snapshot of the list) carries its own time
-- and is applied only when it is newer, or as new and stricter: receiving
-- a copy is not a new decision. 0: nothing was decided here yet.
--
-- `my_contact` gains the value `expired` (the request ran out or was
-- pushed out of the queue): the column is TEXT, no change of the schema.

ALTER TABLE msg_dm_relations ADD COLUMN decided_at INTEGER NOT NULL DEFAULT 0;

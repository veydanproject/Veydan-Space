-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Group calls (messenger-calls, stage 7b). The same table keeps them, one
-- row per call: `kind` tells a call of a group (`group`) from one between
-- two (`dm`); `chat_id` is then the group's chat and `peer` the group id.
-- `started_by` is who made the room (hex), `participants` how many
-- people were seen in it; `direction` is `out` when I started it and
-- `in` otherwise, `answered_at` when I was in the room first, `ended_at`
-- when the call ended for everybody (or when I last left it, if the end
-- never came).

ALTER TABLE msg_calls ADD COLUMN kind TEXT NOT NULL DEFAULT 'dm';
ALTER TABLE msg_calls ADD COLUMN started_by TEXT;
ALTER TABLE msg_calls ADD COLUMN participants INTEGER NOT NULL DEFAULT 0;

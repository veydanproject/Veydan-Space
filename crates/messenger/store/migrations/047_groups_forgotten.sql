-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- The groups I forgot («Remove from this device», the chat of a group
-- deleted), on this device or on another of mine (the snapshot of my
-- relations, `groups_forgotten`): a welcome or keys of the group dated at
-- or before `forgotten_at` bring it back no more — from the archive of the
-- conversation, from a relay, from a welcome to myself another device
-- points to. A welcome after it (a new invitation, the link opened again)
-- is a new entry, and comes in.
--
--   group_id      64 hex
--   forgotten_at  my clock at the forgetting; only ever raised
CREATE TABLE IF NOT EXISTS msg_groups_forgotten (
    group_id     TEXT PRIMARY KEY NOT NULL,
    forgotten_at INTEGER NOT NULL
);

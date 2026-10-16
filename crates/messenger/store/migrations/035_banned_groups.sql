-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- The groups the network closes: `banned_groups` of the manifest in use
-- (internal/messenger-wire.md §1), a group found on a confirmed report to
-- serve the harm of a child. As `msg_banned_keys` (034), the manifest is
-- the truth and this is its copy, kept by `messenger_runtime` whenever the
-- manifest in use changes, so that every gate of the groups and every
-- process that opens the database (the push handler too) read the same
-- list. A table of its own rather than a kind column in 034: a group id
-- is no key, and the predicates that read 034 by `pubkey` stay as they are.
-- An id leaves the table when the manifest stops naming it; nothing the
-- device holds of the group (`msg_groups`, its chat, its keys) is touched
-- by the ban, so a lifted ban leaves the group as it was.

CREATE TABLE IF NOT EXISTS msg_banned_groups (
    group_id TEXT PRIMARY KEY NOT NULL
) WITHOUT ROWID;

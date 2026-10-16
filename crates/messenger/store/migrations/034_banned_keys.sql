-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- The keys the network refuses: `banned_keys` of the manifest in use
-- (internal/messenger-wire.md §1), a person found on a confirmed report to
-- have harmed a child. The manifest is the truth; this is its copy, kept
-- by `messenger_runtime` whenever the manifest in use changes, so that the
-- predicates of the store (what a group shows and counts, the queue of
-- requests) and every process that opens the database (the push handler
-- too) read the same list. A key leaves the table when the manifest stops
-- naming it; the relation stored for it (`msg_dm_relations`) is never
-- touched by the ban, so a lifted ban leaves things as the user had them.

CREATE TABLE IF NOT EXISTS msg_banned_keys (
    pubkey TEXT PRIMARY KEY NOT NULL
) WITHOUT ROWID;

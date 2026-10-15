-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Group rights, version 2 of the operations. An operation newer than
-- this build reads is parked in `msg_group_pending` (bucket `future:…`)
-- until an update; the group row says so, for the screen to ask for one.

ALTER TABLE msg_groups ADD COLUMN needs_update INTEGER NOT NULL DEFAULT 0;

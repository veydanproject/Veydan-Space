-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- The compaction of the archive of the conversation (crate
-- messenger-archive, `drop_dead`) looks for the messages deleted for
-- everyone and the edits of them in `msg_messages`, once per segment it
-- rewrites and once a month over every closed segment. Two partial indexes
-- make those lookups read the few rows they are about, not the table.
CREATE INDEX IF NOT EXISTS msg_messages_deleted ON msg_messages (id, wire_id) WHERE deleted_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS msg_messages_edits ON msg_messages (target_id, id, wire_id) WHERE content_type = 'edit';

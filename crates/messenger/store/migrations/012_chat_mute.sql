-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Whether a chat may make a sound is a setting of the chat, the same for a
-- direct chat and a group, and for a stranger as for a contact. It used to
-- be a setting of a contact, so only contacts could be quiet.

ALTER TABLE msg_chats ADD COLUMN muted INTEGER NOT NULL DEFAULT 0;

UPDATE msg_chats SET muted = 1
 WHERE peer_pubkey IN (SELECT pubkey FROM msg_private_contacts WHERE is_muted = 1);

ALTER TABLE msg_private_contacts DROP COLUMN is_muted;
ALTER TABLE msg_private_contacts DROP COLUMN notification_level;

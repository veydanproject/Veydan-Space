-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- What a chat has shared (pictures, files, links, voice) is read by kind
-- of content, newest first, a page at a time.

CREATE INDEX IF NOT EXISTS msg_messages_chat_content
    ON msg_messages (chat_id, content_type, created_at);

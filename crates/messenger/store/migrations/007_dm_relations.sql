-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Stage 5b: DM relationship state, one row per peer we have any state
-- about (contacts and strangers alike). Owned by the DM module; the
-- address book (`msg_private_contacts`) stays a plain address book.
--
--   my_contact        none | approved | declined     my side
--   blocked           0 | 1                          my block (the only block flag)
--   peer_signal       none | approved | blocked | left | declined | revoked
--   was_ever_mutual   both sides were approved at some point; never reset
--   last_signal_at    rumor time of the newest peer signal applied
--   last_my_signal_at rumor time of my newest action (seen through self-copies)

CREATE TABLE IF NOT EXISTS msg_dm_relations (
    peer_pubkey       TEXT PRIMARY KEY NOT NULL,
    my_contact        TEXT NOT NULL DEFAULT 'none',
    blocked           INTEGER NOT NULL DEFAULT 0,
    peer_signal       TEXT NOT NULL DEFAULT 'none',
    was_ever_mutual   INTEGER NOT NULL DEFAULT 0,
    last_signal_at    INTEGER NOT NULL DEFAULT 0,
    last_my_signal_at INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

-- People already in the address book were added deliberately.
INSERT OR IGNORE INTO msg_dm_relations (peer_pubkey, my_contact, created_at, updated_at)
SELECT pubkey, 'approved', created_at, updated_at FROM msg_private_contacts WHERE deleted_at IS NULL;

-- Conversations that existed before the matrix: whoever wrote to us and
-- got an answer is a mutual contact; chats we only wrote into are requests.
INSERT OR IGNORE INTO msg_dm_relations (peer_pubkey, my_contact, created_at, updated_at)
SELECT c.peer_pubkey, 'approved', c.created_at, c.updated_at FROM msg_chats c
WHERE c.kind = 'dm' AND c.peer_pubkey IS NOT NULL
  AND EXISTS (SELECT 1 FROM msg_messages m WHERE m.chat_id = c.id AND m.direction = 'out' AND m.is_hidden = 0);

UPDATE msg_dm_relations SET peer_signal = 'approved', was_ever_mutual = (my_contact = 'approved')
WHERE EXISTS (
    SELECT 1 FROM msg_messages m
    WHERE m.chat_id = 'dm:' || msg_dm_relations.peer_pubkey AND m.direction = 'in' AND m.is_hidden = 0
);

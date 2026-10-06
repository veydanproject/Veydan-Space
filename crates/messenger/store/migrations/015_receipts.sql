-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Receipts: the peer tells which of my messages reached one of its devices
-- and up to when it read me; I tell the peer the same. What comes is kept
-- by its own key, so it holds whatever the order: a receipt may come
-- before my own copy of the message it names.
--
-- `msg_peer_read`: up to when a member read the chat. In a DM the member is
-- the peer, in a group each member. It only ever moves forward.
--
-- `msg_dm_delivered`: a message of mine that the peer's device has.
--
-- `msg_messages.acked_at`: for an incoming message, when its receipt was
-- queued. NULL means one is still owed; history too old for a receipt
-- gets it set at once.
--
-- `msg_own_read.receipt_due`: the chat was read on this device and the
-- peer has not been told yet. A mark that came from another device of
-- mine does not set it: that device tells the peer itself.
--
-- `msg_messages_unacked`: what is owed is taken every few seconds for the
-- whole session; the take reads the recent incoming DM messages still owed
-- a receipt off this index, not the whole table. Its terms are the take's
-- own (`receipts::OWED_DELIVERED`), or SQLite does not use it.

CREATE TABLE IF NOT EXISTS msg_peer_read (chat_id TEXT NOT NULL, member TEXT NOT NULL, read_at INTEGER NOT NULL, PRIMARY KEY (chat_id, member));
CREATE TABLE IF NOT EXISTS msg_dm_delivered (message_id TEXT PRIMARY KEY NOT NULL, peer TEXT NOT NULL, delivered_at INTEGER NOT NULL);
ALTER TABLE msg_messages ADD COLUMN acked_at INTEGER;
ALTER TABLE msg_own_read ADD COLUMN receipt_due INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS msg_messages_unacked ON msg_messages(created_at) WHERE acked_at IS NULL AND direction = 'in' AND substr(chat_id, 1, 3) = 'dm:';
-- What came before receipts existed owes none: the peers are told of what
-- comes from now on, not of a week of old messages at once.
UPDATE msg_messages SET acked_at = created_at WHERE direction = 'in' AND acked_at IS NULL;

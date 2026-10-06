-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Reactions, and the emoji I use most. Both are kept by their own key, so
-- they hold whatever the order: a reaction may come before the message it
-- names, and is shown once the message is there.
--
-- `msg_reactions`: one row per message, author and emoji. Taking a
-- reaction back keeps the row with `removed = 1`, so an older put that
-- comes later loses to it: the later `created_at` wins. `chat_id` is the
-- chat the reaction came in; it is shown only in that chat, so nobody can
-- put a reaction under a message of a chat they are not in. An author may
-- have more rows standing than are shown (devices that did not know of
-- each other, notes in any order): only the earliest three count, so every
-- device shows the same whatever came first. `kept_at`: when this device
-- first stored the row; one whose message has not come within a week of
-- that is swept, and so is everything of a chat that is deleted.
--
-- `msg_emoji_usage`: how often and when I last used each emoji, on any of
-- my devices. A snapshot from another device raises both to the larger,
-- so all devices come to the same map whatever order snapshots arrive in.

CREATE TABLE IF NOT EXISTS msg_reactions (message_id TEXT NOT NULL, chat_id TEXT NOT NULL, author TEXT NOT NULL, emoji TEXT NOT NULL, created_at INTEGER NOT NULL, removed INTEGER NOT NULL DEFAULT 0, kept_at INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (message_id, author, emoji));
CREATE INDEX IF NOT EXISTS msg_reactions_message ON msg_reactions(message_id, chat_id);
CREATE TABLE IF NOT EXISTS msg_emoji_usage (emoji TEXT PRIMARY KEY NOT NULL, count INTEGER NOT NULL, last_at INTEGER NOT NULL);

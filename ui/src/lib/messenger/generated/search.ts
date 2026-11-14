// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

import type { MessengerMessage } from '../api';

/** Where a page of `messenger_search_messages` ends: its `next_before`, passed as `before` for the next page. */
export type SearchCursor = { created_at: number, id: string, };

/** What the search found in one chat, as `messenger_search_messages` answers: a page, the newest first, and how many match in all; `partial` while the index of this device is still being written. */
export type SearchPage = { 
/**
 * The messages that hold every word, as the chat shows them, the
 * newest first.
 */
messages: MessengerMessage[], 
/**
 * How many messages of the chat match in all.
 */
total: number, 
/**
 * What to pass as `before` for the next page; absent on the last.
 */
next_before?: SearchCursor, 
/**
 * The index does not hold every message of this device yet (it is
 * still being written, after an update): older ones may be missing.
 */
partial: boolean, };

/** A chat whose messages hold the words: how many, and the newest of them. */
export type ChatHit = { chat_id: string, 
/**
 * How many of its messages match.
 */
count: number, 
/**
 * The newest of them: the line the list shows, and where the chat
 * opens.
 */
last: MessengerMessage, };

/** What the search found across the chats, as `messenger_search_chats` answers: the chat with the newest match first. */
export type ChatSearchResult = { 
/**
 * The chat with the newest match first.
 */
hits: Array<ChatHit>, 
/**
 * As in [`SearchPage`].
 */
partial: boolean, };

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Finding a contact or a chat by what the user types. A person is found by
// any of their names, not only by the one the list shows: a nickname hides
// the profile name, and that name is what people remember.

import type { MessengerChat, MessengerContact } from '../api';

/**
 * Lower case without marks: "José" is "jose", "Ёлена" is "елена". So "й"
 * folds to "и" too, which a search can live with.
 */
export function fold(s: string): string {
  return s.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
}

/** Every name and key of a contact, folded; fields stay apart so a word never spans two. */
export function contactHaystack(c: MessengerContact): string {
  const p = c.profile;
  return fold([c.nickname, c.note, p?.display_name, p?.name, p?.nip05, c.npub, c.pubkey].filter(Boolean).join('\n'));
}

/** Haystacks of contacts by pubkey, built once per list rather than per keystroke. */
export function contactHaystacks(contacts: MessengerContact[]): Map<string, string> {
  return new Map(contacts.map((c) => [c.pubkey, contactHaystack(c)]));
}

/** Each word of the query is somewhere in the haystack; an empty query matches all. */
export function matchesQuery(haystack: string, query: string): boolean {
  return fold(query).split(/\s+/).every((w) => !w || haystack.includes(w));
}

export function contactMatches(c: MessengerContact, query: string): boolean {
  return matchesQuery(contactHaystack(c), query);
}

/**
 * A chat by its title, its peer's key and last message, and, for a chat with
 * a contact, by everything that contact is called (`haystacks` from
 * `contactHaystacks`): the title is only the first of those names.
 */
export function chatMatches(chat: MessengerChat, query: string, haystacks: Map<string, string>): boolean {
  if (!query.trim()) return true;
  const own = fold([chat.title, chat.peer_npub, chat.peer_pubkey, chat.last_preview].filter(Boolean).join('\n'));
  const contact = chat.peer_pubkey ? haystacks.get(chat.peer_pubkey) : undefined;
  return matchesQuery(contact ? `${own}\n${contact}` : own, query);
}

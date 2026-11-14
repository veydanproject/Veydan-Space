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

/** What the index of the conversation takes for a character of a word (the runtime's `search::match_query`). */
const WORD = /[\p{L}\p{N}]/u;
/** A mark over the character before it: it goes with that character. */
const MARK = /\p{M}/u;

/** The words of a query as the conversation is searched by them: folded, cut at whatever is no letter or digit. */
export function queryWords(query: string): string[] {
  return fold(query).split(/[^\p{L}\p{N}]+/u).filter(Boolean);
}

/**
 * Where the words of `query` stand in `text` as the search of the
 * conversation finds them: at the start of a word, in any case, with or
 * without marks («ёлк» in «Ёлка», «jose» in «José»). Ranges of `text` as
 * `[from, to)` in its own offsets, in order, never overlapping. The text is
 * folded a character at a time, so each folded character knows the one it
 * came from: a letter and its marks may be one character or several.
 */
export function highlightRanges(text: string, query: string): [number, number][] {
  const words = queryWords(query).map((w) => [...w]);
  if (!words.length || !text) return [];
  const folded: string[] = [];
  /** Where in `text` the character that gave `folded[i]` starts and ends. */
  const from: number[] = [];
  const to: number[] = [];
  let at = 0;
  for (const ch of text) {
    for (const f of fold(ch)) { folded.push(f); from.push(at); to.push(at + ch.length); }
    at += ch.length;
  }
  const found: [number, number][] = [];
  for (let i = 0; i < folded.length; i++) {
    if (i > 0 && WORD.test(folded[i - 1])) continue;
    for (const w of words) {
      if (w.length > folded.length - i || w.some((c, k) => folded[i + k] !== c)) continue;
      let end = to[i + w.length - 1];
      // The marks of the last letter stay with it.
      while (end < text.length && MARK.test(text[end])) end++;
      found.push([from[i], end]);
    }
  }
  found.sort((a, b) => a[0] - b[0] || b[1] - a[1]);
  const out: [number, number][] = [];
  for (const r of found) {
    const last = out[out.length - 1];
    if (last && r[0] <= last[1]) last[1] = Math.max(last[1], r[1]);
    else out.push([...r]);
  }
  return out;
}

/**
 * The line of a match in a list: `text` on one line, begun a few words before
 * the first word found, so a one-line row that cuts its end still shows it;
 * `ranges` as `highlightRanges` gives them, in the offsets of the line.
 */
export function excerpt(text: string, query: string, before = 24): { text: string; ranges: [number, number][] } {
  const line = text.replace(/\s+/g, ' ').trim();
  const ranges = highlightRanges(line, query);
  const first = ranges[0]?.[0] ?? 0;
  if (first <= before) return { text: line, ranges };
  // From the start of a word, not in the middle of one.
  const space = line.indexOf(' ', first - before - 1);
  const cut = space !== -1 && space < first ? space + 1 : first;
  const shift = cut - 1;
  return { text: `…${line.slice(cut)}`, ranges: ranges.map(([a, b]) => [a - shift, b - shift]) };
}

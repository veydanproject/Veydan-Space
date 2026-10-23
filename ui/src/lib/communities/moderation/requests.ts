// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/moderation/requests.ts at 8ce09c39, then changed for channels.

// The requests to be let into a channel as the moderators' tab shows them: their
// order, the search, how long one waits, the facts about the one who
// asks, and the bookkeeping of an act on many at once. Pure, so the tests
// pin it; the tab (RequestsTab.svelte) only draws it.

import type { RequestView } from '../api';

export type RequestOrder = 'newest' | 'oldest';

/** The search field shows once there are more requests than this. */
export const SEARCH_FROM = 5;

/** What this device knows of the one who asks, without asking the network. */
export interface RequestFacts {
  /** The name shown: my nickname for a contact, else theirs, else a short key. */
  name: string;
  picture: string | null;
  nip05: string | null;
  nip05Verified: boolean;
  contact: boolean;
  /** A contact I have a conversation with. */
  talked: boolean;
  /** The first line of what they say about themselves. */
  about: string | null;
  /** Their profile was asked for and never found. */
  missing: boolean;
}

/** By the moment they were made; the key breaks a tie, so the order never jumps. */
export function sortRequests(list: readonly RequestView[], order: RequestOrder): RequestView[] {
  const sign = order === 'newest' ? -1 : 1;
  return [...list].sort((a, b) => sign * (a.created_at - b.created_at) || a.requester.localeCompare(b.requester));
}

/** What a request is found by besides its own fields: the name shown and the checked address. */
export interface RequestWords {
  name: string;
  nip05: string | null;
}

/** Every word typed is somewhere in the name, the key, the address or the note. */
export function matchRequest(query: string, r: RequestView, words: RequestWords): boolean {
  const parts = query.toLocaleLowerCase().split(/\s+/).filter(Boolean);
  if (!parts.length) return true;
  const hay = [words.name, words.nip05 ?? '', r.requester, r.note].join('\n').toLocaleLowerCase();
  return parts.every((p) => hay.includes(p));
}

export function filterRequests(
  list: readonly RequestView[],
  query: string,
  words: (r: RequestView) => RequestWords,
): RequestView[] {
  return query.trim() ? list.filter((r) => matchRequest(query, r, words(r))) : [...list];
}

/** How long a request waits, in the largest whole unit; `null` under a minute. */
export interface Waited {
  unit: 'minute' | 'hour' | 'day';
  n: number;
}

export function waited(received: number, now: number): Waited | null {
  const s = Math.max(0, now - received);
  if (s < 60) return null;
  if (s < 3600) return { unit: 'minute', n: Math.floor(s / 60) };
  if (s < 86400) return { unit: 'hour', n: Math.floor(s / 3600) };
  return { unit: 'day', n: Math.floor(s / 86400) };
}

/** `2 ч`, `3 дн.`, `5 min`: the units of the language, so no plural forms of our own. */
export function waitedText(w: Waited, lang: string): string {
  try {
    return new Intl.NumberFormat(lang, { style: 'unit', unit: w.unit, unitDisplay: 'short' }).format(w.n);
  } catch {
    return `${w.n} ${w.unit}`;
  }
}

const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

/**
 * `Today, 14:05`, `Yesterday, 09:12`, `29 Sep, 14:05`, `29 Sep 2025, 14:05`:
 * a moment with its day, the words for today and yesterday from the caller.
 */
export function cameAt(unix: number, now: Date, lang: string, day: (key: 'today' | 'yesterday') => string): string {
  const d = new Date(unix * 1000);
  const time = d.toLocaleTimeString(lang, { hour: '2-digit', minute: '2-digit' });
  if (sameDay(d, now)) return `${day('today')}, ${time}`;
  const y = new Date(now);
  y.setDate(now.getDate() - 1);
  if (sameDay(d, y)) return `${day('yesterday')}, ${time}`;
  const opts: Intl.DateTimeFormatOptions = { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' };
  if (d.getFullYear() !== now.getFullYear()) opts.year = 'numeric';
  return d.toLocaleString(lang, opts);
}

/** The first line of what they say about themselves, cut to a line's length; `null` when they say nothing. */
export function firstLine(about: string | null | undefined, max = 120): string | null {
  const line = (about ?? '').split('\n').map((l) => l.trim()).find(Boolean);
  if (!line) return null;
  const chars = [...line];
  return chars.length > max ? `${chars.slice(0, max - 1).join('').trimEnd()}…` : line;
}

/** The ticks that still name a request shown. */
export function keepSelected(selected: readonly string[], shown: readonly RequestView[]): string[] {
  const here = new Set(shown.map((r) => r.requester));
  return selected.filter((k) => here.has(k));
}

/** Ticks one more, or unticks it. */
export const toggleSelected = (selected: readonly string[], key: string): string[] =>
  selected.includes(key) ? selected.filter((k) => k !== key) : [...selected, key];

/** All of the list shown ticked: «Выбрать все» then unticks them; otherwise it ticks them, keeping the others ticked. */
export function selectAll(selected: readonly string[], shown: readonly RequestView[]): string[] {
  const keys = shown.map((r) => r.requester);
  if (keys.length && keys.every((k) => selected.includes(k))) return selected.filter((k) => !keys.includes(k));
  return [...new Set([...selected, ...keys])];
}

// ── An act on many ─────────────────────────────────────────────────────

export type BulkKind = 'approve' | 'reject' | 'ban';

export interface BulkFailure {
  requester: string;
  error: string;
}

export interface BulkState {
  kind: BulkKind;
  total: number;
  /** Acts that went through. */
  done: number;
  failed: BulkFailure[];
  running: boolean;
}

/**
 * One act after another through the commands that exist, never two at
 * once (each one changes the channel's journal); a failure is noted and the
 * rest go on. `onprogress` hears every step; the end is returned.
 */
export async function runBulk(
  kind: BulkKind,
  requesters: readonly string[],
  act: (requester: string) => Promise<unknown>,
  explain: (e: unknown) => string,
  onprogress: (s: BulkState) => void = () => {},
): Promise<BulkState> {
  let s: BulkState = { kind, total: requesters.length, done: 0, failed: [], running: true };
  onprogress(s);
  for (const who of requesters) {
    try {
      await act(who);
      s = { ...s, done: s.done + 1 };
    } catch (e) {
      s = { ...s, failed: [...s.failed, { requester: who, error: explain(e) }] };
    }
    onprogress(s);
  }
  s = { ...s, running: false };
  onprogress(s);
  return s;
}

/** How far it is: the acts tried, failed or not. */
export const bulkTried = (s: BulkState): number => s.done + s.failed.length;

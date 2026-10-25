// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The list of the readers of a channel, for its team only (the owner's
// decision, 2026-10-25; `channel_readers`): pages of the core, newest
// first, and a search by name over the readers loaded or by the start of a
// key over them all (the core knows keys, the page knows names).

import type { ReaderView } from './api';

/** Readers one page brings. */
export const READERS_PAGE = 100;
/** The most a refresh reads again at once (the core's bound of a page). */
export const READERS_MAX_PAGE = 500;
/** While shards of the snapshot are on their way, the list is asked again this often, so many times. */
export const PARTIAL_RETRY_MS = 4000;
export const PARTIAL_RETRIES = 8;

/** A query that may be the start of a key, asked of the core: 4 to 64 hex digits; shorter or other words are names. */
export function keyQuery(q: string): string {
  const s = q.trim().toLowerCase();
  return /^[0-9a-f]{4,64}$/.test(s) ? s : '';
}

/** The rows of a further page after those shown, each reader once (a reader who came meanwhile shifts the pages by one). */
export function appendPage(rows: readonly ReaderView[], more: readonly ReaderView[]): ReaderView[] {
  const seen = new Set(rows.map((r) => r.pubkey));
  return [...rows, ...more.filter((r) => !seen.has(r.pubkey))];
}

/**
 * What the list shows for a query: all the rows loaded when it is blank;
 * else those the core found by the start of the key first, then the loaded
 * ones whose name or key `matches`, each once.
 */
export function shownReaders(rows: readonly ReaderView[], hits: readonly ReaderView[], query: string, matches: (pubkey: string) => boolean): ReaderView[] {
  if (!query.trim()) return [...rows];
  return appendPage(hits, rows.filter((r) => matches(r.pubkey)));
}

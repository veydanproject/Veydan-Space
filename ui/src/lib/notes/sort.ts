// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** Client-side ordering shared by the list and table views. Pinned notes always come first. */

import type { NoteListItem } from '$lib/notes/types';

export const SORT_KEYS = ['updated_at', 'created_at', 'title'] as const;
export type SortKey = (typeof SORT_KEYS)[number];

export interface NoteSort {
  key: SortKey;
  asc: boolean;
}

export const DEFAULT_SORT: NoteSort = { key: 'updated_at', asc: false };

const STORAGE_KEY = 'notes-sort';

export function loadSort(): NoteSort {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v) {
      const s = JSON.parse(v) as NoteSort;
      if ((SORT_KEYS as readonly string[]).includes(s.key)) return { key: s.key, asc: !!s.asc };
    }
  } catch {}
  return DEFAULT_SORT;
}

export function saveSort(sort: NoteSort): void {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(sort)); } catch {}
}

/** Fields the ordering needs; both the desktop and the mobile item shapes have them. */
export type Sortable = Pick<NoteListItem, 'pinned' | 'title' | 'updated_at' | 'created_at'>;

function value(n: Sortable, key: SortKey): string {
  return key === 'title' ? n.title.toLowerCase() : n[key];
}

/** Pinned first, then by `sort`. Callers keep FTS relevance order by not calling this. */
export function sortNotes<T extends Sortable>(list: T[], sort: NoteSort): T[] {
  const dir = sort.asc ? 1 : -1;
  return [...list].sort((a, b) => {
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    return value(a, sort.key).localeCompare(value(b, sort.key)) * dir;
  });
}

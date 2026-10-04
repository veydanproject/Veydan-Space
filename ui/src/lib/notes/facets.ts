// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * Facets: namespaced tags `namespace/value` (e.g. `status/production`) read as
 * columns. This is a view over tags, not a properties model; values like ports
 * or dates do not belong in tags.
 */

import type { NoteTagInfo } from '$lib/notes/types';

export interface Facet {
  key: string;
  value: string;
  color: string;
}

/** `namespace/value` tags of a note as facets; plain tags are skipped. */
export function facetsOf(tags: NoteTagInfo[]): Facet[] {
  const out: Facet[] = [];
  for (const t of tags) {
    const i = t.name.indexOf('/');
    if (i <= 0 || i === t.name.length - 1) continue;
    out.push({ key: t.name.slice(0, i), value: t.name.slice(i + 1), color: t.color });
  }
  return out;
}

/** First facet value of `key` on a note. */
export function facetValue(tags: NoteTagInfo[], key: string): Facet | undefined {
  return facetsOf(tags).find((f) => f.key === key);
}

/** Facet keys present in a list of notes, most frequent first. */
export function facetKeys(notes: { tags: NoteTagInfo[] }[], max = 4): string[] {
  const count = new Map<string, number>();
  for (const n of notes) {
    for (const f of facetsOf(n.tags)) count.set(f.key, (count.get(f.key) ?? 0) + 1);
  }
  return [...count.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .slice(0, max)
    .map(([k]) => k);
}

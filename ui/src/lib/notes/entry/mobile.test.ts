// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { afterEach, describe, expect, it } from 'vitest';
import { split as wide } from '$lib/core/ui/split.svelte';
import { module } from './mobile';

const at = (path: string) => new URL(path, 'http://app');
const split = (path: string) => module.split?.(at(path));
const bar = (path: string) => module.bar?.(at(path));

describe('the notes on a wide window (core/ui/split)', () => {
  afterEach(() => {
    wide.on = false;
  });

  it('puts a note and a new one beside the list, the list itself at /notes whatever its filter', () => {
    const list = split('/notes');
    expect(list?.home).toBe('/notes');
    // One list object: the shell keeps it mounted while the filter changes and a note opens.
    for (const path of ['/notes?kind=pinned', '/notes?kind=folder&id=f1', '/notes/n1', '/notes/new', '/notes/new?kind=folder&id=f1']) {
      expect(split(path)).toBe(list);
    }
  });

  it('shows the tags and the search as on a phone', () => {
    expect(split('/notes/tags')).toBeUndefined();
    expect(split('/search')).toBeUndefined();
  });

  it('has no «new» in the column of sections: the «+» of the list stands for it', () => {
    expect(bar('/notes')?.map((i) => i.id)).toEqual(['notes', 'new', 'tags', 'more']);
    wide.on = true;
    expect(bar('/notes')?.map((i) => i.id)).toEqual(['notes', 'tags', 'more']);
    expect(bar('/notes/tags')?.map((i) => i.id)).toEqual(['notes', 'tags', 'more']);
    // The editor keeps the whole height.
    expect(bar('/notes/n1')).toBeNull();
  });
});

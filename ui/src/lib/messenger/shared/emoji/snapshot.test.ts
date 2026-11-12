// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { freeze, shown, takeSnapshot } from './snapshot';

describe('snapshot of popular emoji', () => {
  it('a pick does not change the order of an open row; a new opening does', () => {
    let store = ['a', 'b', 'c'];
    const snap = takeSnapshot(store);
    store = ['c', 'a', 'b']; // used('c') put it first
    expect(shown(snap, store)).toEqual(['a', 'b', 'c']);
    expect(shown(takeSnapshot(store), store)).toEqual(['c', 'a', 'b']);
  });

  it('opened empty, the row follows the store until the first pick, then is frozen', () => {
    let snap = takeSnapshot([], []);
    expect(snap).toBeNull();
    let store = ['x', 'y'];
    expect(shown(snap, store)).toEqual(['x', 'y']);
    snap = freeze(snap, store);
    store = ['y', 'x'];
    expect(shown(snap, store)).toEqual(['x', 'y']);
  });

  it('falls back to local recents when popular is empty', () => {
    expect(takeSnapshot([], ['r'])).toEqual(['r']);
  });
});

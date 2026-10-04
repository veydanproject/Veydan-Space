// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { MAX_PER_ROW, mosaic, mosaicRows, rowHeight } from './mosaic';
import { MAX_ALBUM } from './timeline';

describe('mosaic', () => {
  it('lays out every album size', () => {
    expect([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map(mosaicRows)).toEqual([
      [], [1], [2], [1, 2], [2, 2], [2, 3], [3, 3], [1, 3, 3], [2, 3, 3], [3, 3, 3], [1, 3, 3, 3],
    ]);
  });

  it('places every picture once, in order, never more than three in a row', () => {
    for (let n = 1; n <= MAX_ALBUM; n++) {
      const items = Array.from({ length: n }, (_, i) => i);
      const rows = mosaic(items);
      expect(rows.flat()).toEqual(items);
      expect(rows.every((r) => r.length >= 1 && r.length <= MAX_PER_ROW)).toBe(true);
      // The first picture never has less room than the others.
      expect(rows[0].length).toBeLessThanOrEqual(Math.min(...rows.map((r) => r.length)));
    }
  });

  it('gives fewer pictures taller rows, and a picture alone its own shape', () => {
    expect(rowHeight(1, 1)).toBe(0);
    expect(rowHeight(1, 2)).toBeGreaterThan(rowHeight(2, 2));
    expect(rowHeight(2, 2)).toBeGreaterThan(rowHeight(3, 2));
  });
});

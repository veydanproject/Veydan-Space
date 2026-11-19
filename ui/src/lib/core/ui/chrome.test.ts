// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { chromeBridge, edgesOf } from './chrome';

describe('the edges a strip of the page reaches', () => {
  it('are the UIRectEdge bits of the sides it touches', () => {
    // The strip of the record button: from it to the right edge, above the bottom.
    expect(edgesOf(300, 700, 90, 60, 390, 844)).toBe(8);
    expect(edgesOf(0, 0, 390, 844, 390, 844)).toBe(1 | 2 | 4 | 8);
    expect(edgesOf(-8, 790, 120, 60, 390, 844)).toBe(2 | 4);
    expect(edgesOf(100, 100, 50, 50, 390, 844)).toBe(0);
  });

  it('are none for an empty rectangle', () => {
    expect(edgesOf(0, 0, 0, 0, 390, 844)).toBe(0);
    expect(edgesOf(300, 700, 90, 0, 390, 844)).toBe(0);
  });
});

describe('the bridge', () => {
  it('is none outside a page of a phone', () => {
    expect(chromeBridge()).toBeUndefined();
  });
});

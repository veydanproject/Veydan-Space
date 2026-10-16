// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { anchored, clampView, FIT, percent, stepped, ZOOM_MAX } from './zoom';

const pic = { w: 400, h: 300 };
const screen = { w: 400, h: 600 };

describe('the zoom of the viewer', () => {
  it('never goes below fitting nor above the most', () => {
    expect(clampView({ scale: 0.5, x: 0, y: 0 }, pic, screen).scale).toBe(1);
    expect(clampView({ scale: 20, x: 0, y: 0 }, pic, screen).scale).toBe(ZOOM_MAX);
  });

  it('keeps the picture over the screen where it can fill it', () => {
    // Twice as large: 800 × 600 on 400 × 600, so 200 each way across, none up or down.
    expect(clampView({ scale: 2, x: 999, y: 999 }, pic, screen)).toEqual({ scale: 2, x: 200, y: 0 });
    expect(clampView({ scale: 1, x: 50, y: -50 }, pic, screen)).toEqual(FIT);
  });

  it('zooms about a point: the spot under it stays there', () => {
    const v = anchored(FIT, 2, { x: 100, y: 0 }, { x: 100, y: 0 }, pic, screen);
    expect(v).toEqual({ scale: 2, x: -100, y: 0 });
    // The spot under 100 was 100 from the centre of the picture; at 2 it is 200 from it.
    expect(v.x + 2 * 100).toBe(100);
  });

  it('carries the spot under two fingers as they move', () => {
    const v = anchored({ scale: 2, x: 0, y: 0 }, 2, { x: 0, y: 0 }, { x: 60, y: 0 }, pic, screen);
    expect(v).toEqual({ scale: 2, x: 60, y: 0 });
  });

  it('steps through the levels and names them', () => {
    expect(stepped(1, 1)).toBe(1.5);
    expect(stepped(1.7, 1)).toBe(2);
    expect(stepped(1.7, -1)).toBe(1.5);
    expect(stepped(1, -1)).toBe(1);
    expect(stepped(ZOOM_MAX, 1)).toBe(ZOOM_MAX);
    expect(percent(1.5)).toBe('150%');
  });
});

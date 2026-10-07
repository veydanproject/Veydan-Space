// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The call's capsule stays whole in the window, wherever it was dragged in
// a larger one: on a computer it holds the only End of an audio call.

import { describe, expect, it } from 'vitest';
import { clampPanelShift, PANEL_MARGIN } from './panel';

const card = { width: 300, height: 42, top: 5 };

describe('the place of the call capsule', () => {
  it('keeps a place that fits', () => {
    expect(clampPanelShift({ dx: 120, dy: 40 }, card, { width: 1920, height: 1080 })).toEqual({ dx: 120, dy: 40 });
  });

  it('brings a place kept from a larger window back in', () => {
    // Dragged to the right of a 1920px window, shown in a 1000px one.
    const s = clampPanelShift({ dx: 700, dy: 0 }, card, { width: 1000, height: 700 });
    const left = (1000 - card.width) / 2 + s.dx;
    expect(left + card.width).toBe(1000 - PANEL_MARGIN);
    expect(clampPanelShift({ dx: -900, dy: 0 }, card, { width: 1000, height: 700 }).dx).toBe(PANEL_MARGIN - (1000 - card.width) / 2);
  });

  it('keeps it inside from top to bottom', () => {
    expect(clampPanelShift({ dx: 0, dy: 2000 }, card, { width: 1000, height: 700 }).dy).toBe(700 - PANEL_MARGIN - card.height - card.top);
    expect(clampPanelShift({ dx: 0, dy: -50 }, card, { width: 1000, height: 700 }).dy).toBe(PANEL_MARGIN - card.top);
  });

  it('keeps the top left of a capsule wider than the window in it', () => {
    const s = clampPanelShift({ dx: 300, dy: 0 }, { ...card, width: 500 }, { width: 400, height: 700 });
    expect((400 - 500) / 2 + s.dx).toBe(PANEL_MARGIN);
  });
});

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { DRAG_PX, dismissFor } from './ios-dismiss';

describe('dismissFor', () => {
  it('lets the keyboard go on a drag down', () => {
    expect(dismissFor(0, DRAG_PX + 1, false, false)).toBe(true);
    expect(dismissFor(3, DRAG_PX + 1, false, true)).toBe(true);
  });

  it('keeps it on a drag up or sideways (a swipe to reply)', () => {
    expect(dismissFor(0, -80, false, false)).toBe(false);
    expect(dismissFor(80, DRAG_PX + 5, false, false)).toBe(false);
    expect(dismissFor(0, DRAG_PX - 1, false, false)).toBe(false);
  });

  it('lets it go on a tap, not on a tap of a button or a link', () => {
    expect(dismissFor(2, 3, true, false)).toBe(true);
    expect(dismissFor(2, 3, true, true)).toBe(false);
    expect(dismissFor(2, 3, false, false)).toBe(false);
  });
});

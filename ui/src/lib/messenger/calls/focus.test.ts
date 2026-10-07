// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Tab inside the incoming call's card goes round its buttons and never to
// the page behind it.

import { describe, expect, it } from 'vitest';
import { wrapTab } from './focus';

describe('Tab inside a modal card', () => {
  it('goes from the card itself to its first button, Shift+Tab to its last', () => {
    expect(wrapTab(-1, 3, false)).toBe(0);
    expect(wrapTab(-1, 3, true)).toBe(2);
  });

  it('goes round at the ends instead of leaving the card', () => {
    expect(wrapTab(2, 3, false)).toBe(0);
    expect(wrapTab(0, 3, true)).toBe(2);
  });

  it('leaves the steps inside the card to the browser', () => {
    expect(wrapTab(0, 3, false)).toBeNull();
    expect(wrapTab(2, 3, true)).toBeNull();
  });

  it('keeps the focus where it is when no button can take it', () => {
    expect(wrapTab(-1, 0, false)).toBe(-1);
  });
});

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { FOCUS_RENEW_MS, focusOf, focusToSay } from './focus';

describe('the chat read on this device', () => {
  const reading = { seen: true, focused: true, inMessenger: true, active: 'dm:x' };

  it('is the open chat while it is in sight, in the focus and on the page', () => {
    expect(focusOf(reading)).toBe('dm:x');
    expect(focusOf({ ...reading, active: null })).toBe(null);
    expect(focusOf({ ...reading, seen: false })).toBe(null);
    expect(focusOf({ ...reading, focused: false })).toBe(null);
    expect(focusOf({ ...reading, inMessenger: false })).toBe(null);
  });

  it('is told at a change at once, renewed every half minute, and no chat once', () => {
    expect(focusToSay('dm:x', null, 0, 1_000)).toBe(true);
    expect(focusToSay('dm:y', 'dm:x', 1_000, 1_001)).toBe(true);
    expect(focusToSay(null, 'dm:y', 1_001, 1_002)).toBe(true);
    expect(focusToSay(null, null, 1_002, 1_002 + 10 * FOCUS_RENEW_MS)).toBe(false);
    expect(focusToSay('dm:x', 'dm:x', 1_000, 1_000 + FOCUS_RENEW_MS - 1)).toBe(false);
    expect(focusToSay('dm:x', 'dm:x', 1_000, 1_000 + FOCUS_RENEW_MS)).toBe(true);
  });
});

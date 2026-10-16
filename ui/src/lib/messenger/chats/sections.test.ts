// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { chatKind, inSection } from './sections';

describe('the sections of the chat list', () => {
  it('lists a chat in its own section while nothing is typed', () => {
    expect(chatKind({ kind: 'dm' })).toBe('dm');
    expect(chatKind({ kind: 'group' })).toBe('group');
    // A kind the list does not know is a chat between two.
    expect(chatKind({ kind: 'bot' })).toBe('dm');
    expect(inSection({ kind: 'group' }, 'dm', '')).toBe(false);
    expect(inSection({ kind: 'group' }, 'group', '')).toBe(true);
  });

  it('searches across both sections', () => {
    expect(inSection({ kind: 'group' }, 'dm', 'team')).toBe(true);
    expect(inSection({ kind: 'dm' }, 'group', 'ann')).toBe(true);
  });
});

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { isMine } from './reactions';

describe('my reactions', () => {
  const item = { reactions: [{ emoji: '👍', count: 3, mine: true }, { emoji: '🔥', count: 1, mine: false }] };

  it('are those the runtime marks, and nothing else', () => {
    expect(isMine(item, '👍')).toBe(true);
    expect(isMine(item, '🔥')).toBe(false);
    expect(isMine(item, '❤️')).toBe(false);
    expect(isMine({ reactions: [] }, '👍')).toBe(false);
  });
});

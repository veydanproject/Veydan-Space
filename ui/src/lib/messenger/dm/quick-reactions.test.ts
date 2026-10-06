// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { QUICK_DEFAULTS, quickSet, reactionTarget } from './quick-reactions';

describe('quickSet', () => {
  it('gives the defaults when nothing is popular yet', () => {
    expect(quickSet([])).toEqual([...QUICK_DEFAULTS]);
  });

  it('puts the popular first and fills up with the defaults in order', () => {
    expect(quickSet(['🎉', '🙏'])).toEqual(['🎉', '🙏', '❤️', '👍', '😂', '😮']);
  });

  it('does not repeat a default that is already popular', () => {
    expect(quickSet(['👍', '🎉', '❤️'])).toEqual(['👍', '🎉', '❤️', '😂', '😮', '😢']);
  });

  it('takes six of a long popular list, and skips repeats and empty ones', () => {
    expect(quickSet(['😀', '😀', '', '😎', '🤔', '👀', '🙈', '🚀', '🌟'])).toEqual(['😀', '😎', '🤔', '👀', '🙈', '🚀']);
  });
});

describe('reactionTarget', () => {
  it('is the pressed message when it stands alone', () => {
    expect(reactionTarget('m1')).toBe('m1');
    expect(reactionTarget('m1', [])).toBe('m1');
  });

  it('is the last part of an album, whichever part was pressed', () => {
    const album = ['p1', 'p2', 'p3'];
    for (const pressed of album) expect(reactionTarget(pressed, album)).toBe('p3');
  });
});

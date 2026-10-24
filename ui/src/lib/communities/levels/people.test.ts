// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The people of one level in its dialog (communities-wire §14.1).

import { describe, expect, it } from 'vitest';
import type { ChannelDetail } from '../api';
import { candidates, holders, isHexKey, levelsWith } from './people';

const A = 'a'.repeat(64);
const B = 'b'.repeat(64);
const C = 'c'.repeat(64);
const detail = {
  staff: [
    { pubkey: A, role: 'member', muted: false, trusted: false, since: 1, levels: [0, 1] },
    { pubkey: B, role: 'author', muted: false, trusted: false, since: 2, levels: [0, 1, 2] },
    { pubkey: C, role: 'moderator', muted: false, trusted: false, since: 3 },
  ],
} as unknown as ChannelDetail;

describe('the people of a level', () => {
  it('are those the journal gives it to; the base level has no list', () => {
    expect(holders(detail, 1)).toEqual([A, B]);
    expect(holders(detail, 2)).toEqual([B]);
    expect(holders(detail, 3)).toEqual([]);
    expect(holders(detail, 0)).toEqual([]);
  });

  it('keep the other levels of a person when one is given or taken, and always the base one', () => {
    expect(levelsWith(detail, A, 2, true)).toEqual([0, 1, 2]);
    expect(levelsWith(detail, A, 1, true)).toEqual([0, 1]);
    expect(levelsWith(detail, B, 1, false)).toEqual([0, 2]);
    expect(levelsWith(detail, C, 3, true)).toEqual([0, 3]);
    expect(levelsWith(detail, A, 0, false)).toEqual([0, 1]);
  });

  it('may be given to those known, once each, not to holders, the banned or those not allowed', () => {
    const banned = new Set([C]);
    const D = 'd'.repeat(64);
    const E = 'e'.repeat(64);
    expect(candidates([A, B, C, D, D, E], [A], banned, (w) => w !== E)).toEqual([B, D]);
  });

  it('take a key typed whole in hex', () => {
    expect(isHexKey(` ${A.toUpperCase()} `)).toBe(true);
    expect(isHexKey(A.slice(1))).toBe(false);
    expect(isHexKey('npub1xyz')).toBe(false);
  });
});

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A key the network banned: the runtime refuses it with `peer_banned`, and
// the page must read that as its own text, not as an unknown error.

import { describe, expect, it } from 'vitest';
import { dmErrorCode } from '../api';

describe('the refusal of a banned key', () => {
  it('is read as dm_banned, whatever surrounds the word', () => {
    expect(dmErrorCode(new Error('peer_banned'))).toBe('dm_banned');
    expect(dmErrorCode(new Error('invalid: peer_banned'))).toBe('dm_banned');
  });

  it('leaves the other codes as they were', () => {
    expect(dmErrorCode(new Error('dm_blocked'))).toBe('dm_blocked');
    expect(dmErrorCode(new Error('storage error'))).toBeNull();
  });
});

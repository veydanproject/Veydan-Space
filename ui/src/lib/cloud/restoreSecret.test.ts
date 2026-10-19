// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { emptySecretState, restoreSecretOf } from './restoreSecret';

describe('the secret of the lock in a restore', () => {
  it('takes the one remembered as it is, a PIN by its digits', () => {
    const s = emptySecretState();
    expect(s.mode).toBe('old');
    expect(restoreSecretOf(s)).toBeNull();
    expect(restoreSecretOf({ ...s, old: ' 4821 ' })).toEqual({ password: '4821', kind: 'pin', hint: null, old: true });
    expect(restoreSecretOf({ ...s, old: 'long words' })?.kind).toBe('password');
  });

  it('takes a new one only when it is a secret the lock accepts', () => {
    const s = { ...emptySecretState(), mode: 'new' as const };
    expect(restoreSecretOf({ ...s, password: '4821', confirm: '4822' })).toBeNull();
    expect(restoreSecretOf({ ...s, password: '4821', confirm: '4821', hint: ' cat ' })).toEqual({ password: '4821', kind: 'pin', hint: 'cat', old: false });
  });
});

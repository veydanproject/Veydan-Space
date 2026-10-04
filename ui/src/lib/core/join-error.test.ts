// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { ERR_JOIN_OWN_LOCK, joinErrorKey } from './join-error';

describe('joinErrorKey', () => {
  it('knows the text the lock crate refuses a join with', () => {
    const source = readFileSync(new URL('../../../../crates/lock/src/vault.rs', import.meta.url), 'utf8');
    expect(source).toContain(`pub const ERR_JOIN_OWN_LOCK: &str = ${JSON.stringify(ERR_JOIN_OWN_LOCK)};`);
  });

  it('translates only the refusal', () => {
    expect(joinErrorKey({ code: 'other', message: ERR_JOIN_OWN_LOCK })).toBe('settings_sync_join_own_lock');
    expect(joinErrorKey({ code: 'other', message: 'wrong passphrase' })).toBeNull();
    expect(joinErrorKey({ code: 'db', message: ERR_JOIN_OWN_LOCK })).toBeNull();
    expect(joinErrorKey('boom')).toBeNull();
    expect(joinErrorKey(null)).toBeNull();
  });
});

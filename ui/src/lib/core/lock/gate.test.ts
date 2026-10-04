// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { gateScreen, recoveryStep } from './gate';

const unlocked = { enabled: true, locked: false, timeout_min: 5, vault: 'none', kind: 'pin', hint: null, has_recovery: true };
const lockCalls = vi.hoisted(() => ({
  recover: vi.fn(),
  lock: vi.fn(),
}));
vi.mock('$lib/core/api', () => ({ api: { lock: lockCalls } }));

const { appLock } = await import('./store.svelte');

describe('the lock gate', () => {
  it('keeps the recovery flow up after the recovery unlocked the app', () => {
    // Step 3 (the new recovery key) comes after `recover()` reported "unlocked".
    expect(gateScreen({ ready: true, locked: false, recovering: true, keyPending: true })).toBe('recover');
    // The flow is closed but its key was not confirmed: still the flow.
    expect(gateScreen({ ready: true, locked: false, recovering: false, keyPending: true })).toBe('recover');
  });

  it('shows the app once the flow is done, and the lock screen while locked', () => {
    expect(gateScreen({ ready: true, locked: false, recovering: false, keyPending: false })).toBe('app');
    expect(gateScreen({ ready: true, locked: true, recovering: false, keyPending: false })).toBe('lock');
    expect(gateScreen({ ready: true, locked: true, recovering: true, keyPending: false })).toBe('recover');
    expect(gateScreen({ ready: false, locked: false, recovering: false, keyPending: false })).toBe('blank');
  });

  it('never shows a pending key on a locked screen', () => {
    expect(gateScreen({ ready: true, locked: true, recovering: false, keyPending: true })).toBe('lock');
    expect(recoveryStep({ locked: true, newKey: 'KEY' })).toBe(1);
    expect(recoveryStep({ locked: false, newKey: 'KEY' })).toBe(3);
    expect(recoveryStep({ locked: false, newKey: null })).toBe(1);
  });
});

describe('the recovery in the lock store', () => {
  beforeEach(() => {
    appLock.finishRecovery();
    appLock.status = { ...unlocked, locked: true } as typeof appLock.status;
  });

  it('keeps the app gated from the recovery until step 3 is confirmed', async () => {
    lockCalls.recover.mockResolvedValueOnce({ ...unlocked, recovery_key: 'AAAA-BBBB' });
    appLock.recovering = true;
    expect(appLock.gated).toBe(true);
    await appLock.recover('OLD', { password: '1234', kind: 'pin', hint: null });
    // The backend says unlocked; the shells still show nothing of the app.
    expect(appLock.locked).toBe(false);
    expect(appLock.gated).toBe(true);
    expect(appLock.newRecoveryKey).toBe('AAAA-BBBB');
    appLock.finishRecovery();
    expect(appLock.gated).toBe(false);
    expect(appLock.newRecoveryKey).toBeNull();
  });

  it('keeps the new key through a lock before Done', async () => {
    lockCalls.recover.mockResolvedValueOnce({ ...unlocked, recovery_key: 'CCCC-DDDD' });
    lockCalls.lock.mockResolvedValueOnce({ ...unlocked, locked: true });
    appLock.recovering = true;
    await appLock.recover('OLD', { password: '1234', kind: 'pin', hint: null });
    await appLock.lock();
    // What AppLockGate does on a lock: the flow closes, the key stays.
    appLock.recovering = false;
    expect(gateScreen({ ready: true, locked: appLock.locked, recovering: appLock.recovering, keyPending: appLock.newRecoveryKey !== null })).toBe('lock');
    appLock.status = { ...unlocked } as typeof appLock.status;
    expect(gateScreen({ ready: true, locked: appLock.locked, recovering: appLock.recovering, keyPending: appLock.newRecoveryKey !== null })).toBe('recover');
    expect(recoveryStep({ locked: appLock.locked, newKey: appLock.newRecoveryKey })).toBe(3);
    expect(appLock.newRecoveryKey).toBe('CCCC-DDDD');
  });
});

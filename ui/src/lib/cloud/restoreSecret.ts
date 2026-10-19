// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The secret of the lock a restore takes, as the screen holds it
// (cloud-recovery.md §7): asked only of an account with a lock, and then
// the one the person remembers or a new one.

import type { CloudRestoreSecret } from '$lib/cloud/api';
import { lockSecretIssue } from '$lib/core/lock/secret';
import type { LockKind } from '$lib/core/types';

/** `old`: the person remembers the PIN or password of the lock; `new`: sets another. */
export type RestoreSecretMode = 'old' | 'new';

export interface RestoreSecretState {
  mode: RestoreSecretMode;
  /** What the person remembers. */
  old: string;
  kind: LockKind;
  password: string;
  confirm: string;
  hint: string;
}

export const emptySecretState = (): RestoreSecretState => ({ mode: 'old', old: '', kind: 'pin', password: '', confirm: '', hint: '' });

/** The secret to send, or null while what is typed is not one yet. */
export function restoreSecretOf(s: RestoreSecretState): CloudRestoreSecret | null {
  if (s.mode === 'old') {
    const old = s.old.trim();
    if (!old) return null;
    // Kept as it is when it opens the row; set as the secret without the vault.
    return { password: old, kind: /^\d+$/.test(old) ? 'pin' : 'password', hint: null, old: true };
  }
  if (lockSecretIssue(s.kind, s.password, s.confirm) !== null) return null;
  return { password: s.password.trim(), kind: s.kind, hint: s.hint.trim() || null, old: false };
}

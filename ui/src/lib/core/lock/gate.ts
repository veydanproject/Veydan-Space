// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * What the lock gate shows. A recovery unlocks the app before its last step:
 * `appLock.recover()` returns the new status (unlocked) and the new recovery
 * key together, and the key is shown once, on step 3. The gate stays up while
 * the recovery flow is open or its key is not confirmed (`appLock.gated`), so
 * that step is seen; the flow's Done opens the app. A lock before Done shows
 * the PIN screen: the key is not shown on a locked screen.
 */
export function gateScreen(s: {
  ready: boolean;
  locked: boolean;
  recovering: boolean;
  /** A recovery issued a key that step 3 has not had confirmed yet. */
  keyPending: boolean;
}): 'blank' | 'lock' | 'recover' | 'app' {
  if (!s.ready) return 'blank';
  if (s.locked) return s.recovering ? 'recover' : 'lock';
  if (s.recovering || s.keyPending) return 'recover';
  return 'app';
}

/**
 * The step the recovery flow opens on: the new key's (3) when a recovery
 * issued one and the app is unlocked — after a lock that came before Done —,
 * otherwise the first.
 */
export function recoveryStep(s: { locked: boolean; newKey: string | null }): 1 | 3 {
  return s.newKey !== null && !s.locked ? 3 : 1;
}

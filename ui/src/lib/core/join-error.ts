// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { AppError } from '$lib/core/types';
import type { TranslationKey } from '$lib/core/i18n';

/**
 * Mirrors `ERR_JOIN_OWN_LOCK` in crates/lock/src/vault.rs. The refusal has no
 * code of its own: it arrives as `other` with exactly this text.
 */
export const ERR_JOIN_OWN_LOCK =
  'Passwords or messenger keys on this device are behind the lock it has now: one set here, or that of a vault it was connected to before. Turn the lock off on this device, then connect to the vault again.';

/** User-facing text for a refused vault join, or null when it is not one. */
export function joinErrorKey(e: unknown): TranslationKey | null {
  if (e == null || typeof e !== 'object') return null;
  const { code, message } = e as AppError;
  if (code === 'other' && message === ERR_JOIN_OWN_LOCK) return 'settings_sync_join_own_lock';
  return null;
}

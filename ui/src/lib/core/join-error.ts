// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { AppError } from '$lib/core/types';
import type { TranslationKey } from '$lib/core/i18n';
import { cloudNotReady } from '$lib/core/sync/cloud';

/**
 * Mirrors `ERR_JOIN_OWN_LOCK` in crates/lock/src/vault.rs. The refusal has no
 * code of its own: it arrives as `other` with exactly this text.
 */
export const ERR_JOIN_OWN_LOCK =
  'Passwords or messenger keys on this device are behind the lock it has now: one set here, or that of a vault it was connected to before. Turn the lock off on this device, then connect to the vault again.';

/**
 * Mirror `ERR_NO_ATOMIC_MOVE` and `ERR_NO_CONDITIONAL_PUT` in
 * crates/sync/src/storage.rs: a storage that cannot make a write atomic or
 * conditional is refused. They arrive inside a longer text (`storage: …`),
 * from a command or as the last error of the cycle.
 */
export const ERR_NO_ATOMIC_MOVE = 'storage does not support atomic writes (MOVE)';
export const ERR_NO_CONDITIONAL_PUT = 'storage does not support conditional writes (If-None-Match)';

/**
 * Mirrors `ERR_VAULT_UNREAD` in crates/sync-host/src/join.rs: a join reads the
 * whole vault and is refused while it is not all there yet. The cause follows
 * the text.
 */
export const ERR_VAULT_UNREAD = 'the vault could not be read in full, try again later';

/** Mirrors `ERR_SYNC_RUNNING` in crates/sync-host/src/join.rs: a join, a create or a leave while a cycle runs. */
export const ERR_SYNC_RUNNING = 'a sync is running: wait until it ends, then try again';

/** User-facing text for a storage the sync refuses, or null when the message is not one. */
export function storageErrorKey(message: string | null | undefined): TranslationKey | null {
  if (!message) return null;
  if (message.includes(ERR_NO_ATOMIC_MOVE)) return 'settings_sync_storage_no_move';
  if (message.includes(ERR_NO_CONDITIONAL_PUT)) return 'settings_sync_storage_no_conditional';
  return null;
}

/** User-facing text for a refused vault join, or null when it is not one. */
export function joinErrorKey(e: unknown): TranslationKey | null {
  if (e == null || typeof e !== 'object') return null;
  const { code, message } = e as AppError;
  if (code === 'other' && message === ERR_JOIN_OWN_LOCK) return 'settings_sync_join_own_lock';
  // The storage of Veydan Cloud does not know the account yet: the words of the wait, not of the host.
  if (cloudNotReady(e)) return 'settings_sync_cloud_not_ready';
  // A restore of Veydan Cloud carries these refusals as errors of the storage (`CloudError`).
  const ours = code === 'other' || (code as string) === 'storage';
  if (ours && message?.startsWith(ERR_VAULT_UNREAD)) return 'settings_sync_join_unread';
  if (ours && message === ERR_SYNC_RUNNING) return 'settings_sync_busy';
  if (code === 'other') return storageErrorKey(message);
  return null;
}

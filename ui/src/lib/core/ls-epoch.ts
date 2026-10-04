// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The webview data directory survives a major version, so localStorage written
// by an older UI reaches this one. Keys of another epoch are dropped as a whole.

export const LS_EPOCH_KEY = 'veydan_ls_epoch';
export const LS_EPOCH = '5';

/** Clears `storage` unless it already belongs to this epoch. Must run before any other key is read. */
export function applyLsEpoch(storage: Storage): void {
  if (storage.getItem(LS_EPOCH_KEY) === LS_EPOCH) return;
  storage.clear();
  storage.setItem(LS_EPOCH_KEY, LS_EPOCH);
}

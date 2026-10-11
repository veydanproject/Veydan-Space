// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The vault of sync as «Настроить восстановление» finds it on this device
// (internal/cloud-recovery.md §4): the copy for recovery carries the key
// of the vault and where it lives, so there must be one. A device that
// syncs nowhere gets the vault of Veydan Cloud with the setup; a storage
// of the user's own that is chosen but not connected is finished in the
// settings of sync, never replaced.

import { api, type SyncConfig, type SyncStatus } from '$lib/core/api';

// The vault of Veydan Cloud is turned on by the core's own words: the first start does the same.
export { connectCloudVault } from '$lib/core/sync/cloud';

/**
 * `joined`: the device syncs with a vault. `none`: it syncs nowhere, and
 * the setup turns on Veydan Cloud. `own`: the user chose a storage of
 * their own and has not connected it yet.
 */
export type VaultState = 'joined' | 'none' | 'own';

export function vaultState(status: Pick<SyncStatus, 'joined'> | null, cfg: SyncConfig | null): VaultState {
  if (status?.joined) return 'joined';
  if (!cfg) return 'none';
  switch (cfg.backend) {
    case 'folder':
      return cfg.folder_path.trim() ? 'own' : 'none';
    case 's3':
      return cfg.s3.endpoint.trim() || cfg.s3.bucket.trim() ? 'own' : 'none';
    case 'webdav':
      return cfg.webdav.url.trim() ? 'own' : 'none';
    default:
      return 'none';
  }
}

/** The vault of sync of this device, as its sync says now; `joined` where the product has no sync to ask. */
export async function loadVaultState(): Promise<VaultState> {
  try {
    return vaultState(await api.sync.status(), await api.sync.getConfig());
  } catch {
    return 'joined';
  }
}


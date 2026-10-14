// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Commands of the backup service of Space (platform-spec 1.2, 11.3): the
// copy of the data folder, its schedule and the restore.

import { api as core, call } from '$lib/core/api';
import type { BackupConfig, BackupFileInfo } from '$lib/backup/types';

export const api = {
  ...core,
  backup: {
    getConfig: () => call<BackupConfig>('backup_get_config'),
    setConfig: (cfg: BackupConfig) => call<void>('backup_set_config', { cfg }),
    list: () => call<BackupFileInfo[]>('backup_list'),
    runNow: () => call<void>('backup_run_now'),
    /**
     * Empty password = use the configured one. An archive older than the
     * newest one of the data fails with `backup_older:<its date>|<the newest>`
     * unless `acceptOlder` (read it with `olderBackup`).
     */
    restore: (path: string, password: string, acceptOlder = false) =>
      call<void>('backup_restore', { path, password: password || null, acceptOlder }),
  },
};

/** The dates of a restore refused as older than the data, if the error is one. */
export function olderBackup(message: string): { created: string; newest: string } | null {
  const m = /^backup_older:([^|]*)\|(.*)$/.exec(message);
  return m ? { created: m[1], newest: m[2] } : null;
}

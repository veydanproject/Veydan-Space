// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Payloads of the backup service's commands.

export interface BackupConfig {
  dir: string | null;
  /** Not returned by the backend. On save: null = keep stored, '' = clear. */
  password?: string | null;
  has_password: boolean;
  schedule_enabled: boolean;
  schedule_mode: 'interval' | 'daily' | 'weekly';
  interval_hours: number;
  time: string;
  weekday: number;
  keep: number;
  last_run: string | null;
  /** What the last restore came to; read after the restart, never saved. */
  last_restore?: RestoreOutcome | null;
}

/** The outcome of the last restore of this data folder (backup_get_config). */
export interface RestoreOutcome {
  at: string;
  /** failed: the swap failed and was undone; undone or finished: it was cut short and the next start undid or finished it. */
  result: 'restored' | 'failed' | 'undone' | 'finished';
  error?: string;
  /** The previous folders outside the data folder, kept beside theirs. */
  kept?: string[];
}

export interface BackupFileInfo {
  name: string;
  path: string;
  size: number;
  modified: string;
}

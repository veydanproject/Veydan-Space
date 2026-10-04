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
}

export interface BackupFileInfo {
  name: string;
  path: string;
  size: number;
  modified: string;
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The backup service as the desktop shell sees it: its card on the settings
// page, in the Data block before Sync.

import type { ModuleDef } from '$lib/core/module';
import { common } from './common';
import BackupCard from '$lib/backup/desktop/BackupCard.svelte';

export const module: ModuleDef = {
  ...common,
  nav: [],
  settings: [
    { id: 'backup', title: 'settings_backup_section', group: 'settings_group_data', order: 50, hint: 'settings_backup_hint', component: BackupCard },
  ],
};

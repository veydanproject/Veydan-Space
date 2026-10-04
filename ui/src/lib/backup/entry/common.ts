// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The backup service of Space regardless of the shell (platform-spec 1.2,
// 11.3): a service, not a module the user picks — no navigation, no switch of
// its own (section 12). It exists on the desktop only; there is no mobile entry.

import type { ModuleDef } from '$lib/core/module';

export const common: Omit<ModuleDef, 'nav' | 'settings'> = {
  id: 'backup',
  title: 'settings_backup_section',
  icon: 'archive',
  routes: [],
  service: true,
};

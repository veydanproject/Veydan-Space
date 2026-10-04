// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the ssh module is regardless of the shell (platform-spec 11.1).
// The module exists on the desktop only (1.2); there is no mobile entry.

import type { ModuleDef } from '$lib/core/module';
import { sshEntities } from '$lib/ssh/entities';
import { sshStore } from '$lib/ssh/store/ssh.svelte';

export const common: Omit<ModuleDef, 'nav' | 'settings'> = {
  id: 'ssh',
  title: 'nav_terminal',
  icon: 'terminal',
  routes: ['/terminal', '/files'],
  entities: sshEntities,
  demo: true,
  reloaders: {
    ssh_connection: () => sshStore.loadConnections(),
    ssh_key: () => sshStore.loadConnections(),
  },
  async start() {
    void sshStore.ensureLoaded();
  },
};

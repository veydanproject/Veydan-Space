// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the pass module is regardless of the shell (platform-spec 11.1).

import type { ModuleDef } from '$lib/core/module';
import { api } from '$lib/pass/api';
import { passEntities } from '$lib/pass/entities';
import { passwordStore } from '$lib/pass/store/passwords.svelte';
import { totpStore } from '$lib/pass/store/totp.svelte';

export const common: Omit<ModuleDef, 'nav' | 'settings'> = {
  id: 'pass',
  title: 'pw_title',
  icon: 'lock',
  routes: ['/passwords', '/totp', '/tools'],
  entities: passEntities,
  demo: true,
  reloaders: {
    totp: () => totpStore.loaded ? totpStore.refresh() : Promise.resolve(),
    password: () => passwordStore.loaded ? passwordStore.refresh() : Promise.resolve(),
    password_vault: async () => {
      const { appLock } = await import('$lib/core/lock/store.svelte');
      await appLock.refresh();
      if (passwordStore.loaded) await passwordStore.refresh();
    },
  },
  // The lock's last resort: the passwords go, the lock gets a new key (section 8).
  vaultReset: (secret) => api.passwords.vaultReset(secret),
};

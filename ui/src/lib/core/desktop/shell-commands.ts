// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The shell's own commands of the palette: every product has them, whatever
// its modules register (a product whose module registers none still has a
// palette that does something).

import { get } from 'svelte/store';
import { goto } from '$app/navigation';
import { commands } from '$lib/core/commands';
import { t } from '$lib/core/i18n';
import { appLock } from '$lib/core/lock/store.svelte';
import { updaterStore } from '$lib/core/store/updater.svelte';
import { toggleTheme } from '$lib/core/theme';

let registered = false;

/** Registered once by the desktop shell; a provider, so titles follow the locale. */
export function registerShellCommands(): void {
  if (registered) return;
  registered = true;
  commands.addProvider(() => {
    const tr = get(t);
    const group = tr('hotkey_group_app');
    return [
      {
        id: 'app.settings',
        title: tr('cmd_app_settings'),
        keywords: 'settings preferences options',
        icon: 'settings',
        group,
        run: () => goto('/settings'),
      },
      {
        id: 'app.theme',
        title: tr('theme_toggle'),
        keywords: 'theme dark light',
        icon: 'moon',
        group,
        run: toggleTheme,
      },
      {
        id: 'app.lock',
        title: tr('cmd_app_lock'),
        keywords: 'lock pin password',
        icon: 'lock',
        group,
        when: () => appLock.status.enabled,
        run: () => appLock.lock(),
      },
      {
        id: 'app.updates',
        title: tr('settings_update_check'),
        keywords: 'update version release',
        icon: 'download',
        group,
        run: async () => {
          await goto('/settings#updates');
          await updaterStore.check(false);
        },
      },
    ];
  });
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** The pass module's commands for the palette (`ModuleDef.commands`), registered once by the shell. */

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { commands } from '$lib/core/commands';
import { passwordStore } from '$lib/pass/store/passwords.svelte';
import { showPass } from '$lib/pass/desktop/show';

const tr = () => get(t);

let registered = false;

export function registerPassCommands(): void {
  // The shell may remount (HMR, window reopen); providers must not stack up
  if (registered) return;
  registered = true;
  // Provider rather than static registration so titles follow the current locale
  commands.addProvider(() => [
    {
      id: 'passwords.create',
      title: tr()('cmd_passwords_create'),
      keywords: 'new password',
      icon: 'plus',
      group: tr()('ctx_kind_password'),
      run: () => {
        passwordStore.createRequest = true;
        showPass('passwords');
      },
    },
  ]);
}

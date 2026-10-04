// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the messenger module is regardless of the shell (platform-spec 11.1).
// The module imports from the core only the types of this description, the
// ui primitives, i18n and Icon (6.3), so it stays extractable.

import type { ModuleDef } from '$lib/core/module';
import { messengerStore } from '../store.svelte';
import { BASE } from '../mobile/routes';

export const common: Omit<ModuleDef, 'nav' | 'settings'> = {
  id: 'messenger',
  title: 'nav_messenger',
  icon: 'message-circle',
  routes: [BASE],
  // The messenger's key on this device is kept under the lock's key: a vault reset loses it too.
  vaultResetNote: 'pw_reset_confirm_messenger',
};

/** The one navigation entry: shown while the module is switched on and its runtime started. */
export const navItem = {
  id: 'messenger',
  title: 'nav_messenger',
  icon: 'message-circle',
  href: BASE,
  visible: () => messengerStore.visible,
  badge: () => messengerStore.unread,
} as const;

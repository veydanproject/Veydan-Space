// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the messenger module is regardless of the shell (platform-spec 11.1).
// The module imports from the core only the types of this description, the
// ui primitives, i18n and Icon (6.3), so it stays extractable.

import type { AccountLink, ModuleDef } from '$lib/core/module';
import { profileLabel } from '../api';
import { messengerStore } from '../store.svelte';
import { BASE } from '../mobile/routes';
import AccountStep from '../onboarding/AccountStep.svelte';
import ProfileStep from '../onboarding/ProfileStep.svelte';

/** The key and the servers are here: the account of this device is whole. */
async function hasAccount(): Promise<boolean> {
  await messengerStore.ensureLoaded().catch(() => {});
  return !!messengerStore.identity && !messengerStore.pendingBackup && !!messengerStore.serversMode;
}

/**
 * The settings of the messenger, where the profile, the servers and the
 * copy of the key are: a screen of its own on a phone, a pane of the page
 * on a computer.
 */
export function accountLinks(settingsHref: string): () => Promise<AccountLink[]> {
  return async () => {
    if (!(await hasAccount())) return [];
    return [
      { id: 'profile', title: 'msg_account_profile', icon: 'user', href: settingsHref },
      {
        id: 'servers',
        title: 'msg_account_servers',
        icon: 'server-2',
        href: settingsHref,
        value: messengerStore.serversMode === 'own' ? 'msg_ob_servers_own' : 'msg_ob_servers_veydan',
      },
      { id: 'key', title: 'msg_account_key', icon: 'download', href: settingsHref },
    ];
  };
}

export const common: Omit<ModuleDef, 'nav' | 'settings'> = {
  id: 'messenger',
  title: 'nav_messenger',
  icon: 'message-circle',
  routes: [BASE],
  // The messenger's key on this device is kept under the lock's key: a vault reset loses it too.
  vaultResetNote: 'pw_reset_confirm_messenger',
  // The first start: the account, after the modules and before sync, then its name and picture once it is there.
  onboarding: [
    { id: 'account', title: 'msg_ob_step', order: 20, component: AccountStep },
    {
      id: 'profile',
      title: 'msg_ob_profile_step',
      order: 25,
      component: ProfileStep,
      visible: () => !!messengerStore.identity && !messengerStore.pendingBackup && !!messengerStore.serversMode,
    },
  ],
  // Passed by at the first start, the account waits on the card «Finish setting up».
  setup: [{ id: 'account', order: 10, title: 'msg_setup_account', icon: 'user', href: () => BASE, step: 'account', done: hasAccount }],
};

/** The messenger owns the key of the account: who it is, for the settings and the page «Account». */
export function accountOf(settingsHref: string): NonNullable<ModuleDef['account']> {
  return {
    profile: async () => {
      await messengerStore.ensureLoaded().catch(() => {});
      const identity = messengerStore.identity;
      return identity ? { npub: identity.npub, name: profileLabel(messengerStore.ownProfile) } : null;
    },
    createHref: BASE,
    links: accountLinks(settingsHref),
  };
}

/** The one navigation entry: shown while the module is switched on and its runtime started. */
export const navItem = {
  id: 'messenger',
  title: 'nav_messenger',
  icon: 'message-circle',
  href: BASE,
  visible: () => messengerStore.visible,
  badge: () => messengerStore.unread,
} as const;

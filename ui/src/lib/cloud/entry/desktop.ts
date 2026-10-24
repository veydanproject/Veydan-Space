// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The Cloud module as the desktop shell sees it: the page «Account» as the
// first section of the settings, the same page `/cloud` shows.

import type { ModuleDef } from '$lib/core/module';
import { common } from './common';
import SettingsAccount from '$lib/cloud/SettingsAccount.svelte';
import Banners from '$lib/cloud/Banners.svelte';

export const module: ModuleDef = {
  ...common,
  nav: [],
  // Before the core's first block; the page draws its own cards.
  settings: [{ id: 'account', title: 'cloud_account_title', group: 'cloud_account_title', order: 1, icon: 'user', component: SettingsAccount, bare: true }],
  // Over any page, in one column: the cards of the bot about recovery («Это не я — отменить», cloud-recovery.md §10) and the room of the vault that runs out, with the ways out.
  overlays: [{ id: 'banners', component: Banners, place: 'window' }],
};

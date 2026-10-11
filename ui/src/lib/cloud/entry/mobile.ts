// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The Cloud module as the mobile shell sees it: the account at the top of
// the settings, which leads to the page «Account» (`/cloud`) and to sync.

import type { ModuleDef } from '$lib/core/module';
import { common } from './common';
import AccountRows from '$lib/cloud/AccountRows.svelte';
import Banners from '$lib/cloud/Banners.svelte';

export const module: ModuleDef = {
  ...common,
  nav: [],
  settings: [{ id: 'account', title: 'cloud_account_title', group: 'cloud_account_title', order: 1, component: AccountRows, into: 'account' }],
  // Over any page, in one column: the cards of the bot about recovery («Это не я — отменить», cloud-recovery.md §10) and the room of the vault that runs out, with the ways out.
  overlays: [{ id: 'banners', component: Banners, place: 'window' }],
};

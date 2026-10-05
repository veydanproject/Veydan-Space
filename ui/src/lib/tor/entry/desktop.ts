// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The Tor module as the desktop shell sees it: its card on the settings page,
// in the Browser block after Camoufox. The page behind the card is `/tor`.

import type { ModuleDef } from '$lib/core/module';
import { common } from './common';
import TorCard from '$lib/tor/desktop/TorCard.svelte';

export const module: ModuleDef = {
  ...common,
  nav: [],
  settings: [
    { id: 'tor', title: 'settings_tor_title', group: 'settings_group_browser', order: 20, hint: 'settings_tor_hint', component: TorCard },
  ],
};

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The Tor module as the desktop shell sees it: its section of the settings, in
// the Browser block after Camoufox, which is the whole of Tor (its state, its
// install and its settings); the address `/tor` leads to it.

import type { ModuleDef } from '$lib/core/module';
import { common } from './common';
import TorSection from '$lib/tor/desktop/TorSection.svelte';

export const module: ModuleDef = {
  ...common,
  nav: [],
  settings: [
    { id: 'tor', title: 'settings_tor_title', group: 'settings_group_browser', order: 20, hint: 'settings_tor_hint', icon: 'globe', component: TorSection, bare: true },
  ],
};

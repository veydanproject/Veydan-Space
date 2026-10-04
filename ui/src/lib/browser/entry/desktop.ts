// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The browser module as the desktop shell sees it (platform-spec 11.1).

import { get } from 'svelte/store';
import type { ModuleDef } from '$lib/core/module';
import { t } from '$lib/core/i18n';
import { common } from './common';
import HomePage from '$lib/browser/desktop/HomePage.svelte';
import RunningDock from '$lib/browser/desktop/RunningDock.svelte';
import CamoufoxCard from '$lib/browser/desktop/CamoufoxCard.svelte';
import NoteCaptureRules from '$lib/browser/components/NoteCaptureRules.svelte';
import ProxyEditorOverlay from '$lib/browser/desktop/ProxyEditorOverlay.svelte';

export const module: ModuleDef = {
  ...common,
  nav: [
    { id: 'workspaces', title: 'nav_workspaces', icon: 'layers', href: '/' },
    { id: 'proxies', title: 'nav_proxies', icon: 'globe', href: '/proxies' },
  ],
  settings: [
    { id: 'camoufox', title: 'settings_camoufox_title', group: 'settings_group_browser', order: 10, component: CamoufoxCard },
    // The web clipper writes into notes; its rules belong to Space's capture module, shown with the notes' cards.
    { id: 'capture', title: 'settings_capture_section', group: 'settings_group_notes', order: 32, hint: 'settings_capture_hint', component: NoteCaptureRules },
  ],
  home: HomePage,
  // The tray's running profiles, the launcher submenu and the sections it opens.
  tray: () => {
    const tt = get(t);
    return {
      running: tt('tray_running'),
      stop_all: tt('tray_stop_all'),
      no_running: tt('tray_no_running'),
      launch_profile: tt('tray_launch_profile'),
      no_profiles: tt('tray_no_profiles'),
      section_workspaces: tt('nav_workspaces'),
      section_proxies: tt('nav_proxies'),
    };
  },
  overlays: [
    { id: 'dock', component: RunningDock, place: 'bottom', order: 20 },
    { id: 'proxy-editor', component: ProxyEditorOverlay, place: 'window' },
  ],
};

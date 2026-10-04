// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The messenger as the mobile shell sees it (platform-spec 11.1).

import type { ModuleDef } from '$lib/core/module';
import { common, navItem } from './common';
import { messengerStore } from '../store.svelte';
import { followRoute, launchRoute, startPushBridge } from '../push/bridge';
import { BASE } from '../mobile/routes';
import NoticeBanner from '../push/NoticeBanner.svelte';

export const module: ModuleDef = {
  ...common,
  nav: [navItem],
  // The module's switch is in the Modules section of the core (section 12).
  settings: [],
  overlays: [{ id: 'notices', component: NoticeBanner, place: 'window' }],
  // The chat, contacts and settings screens use the whole height; the list keeps the root bar.
  bar: (url) => (url.pathname.startsWith(`${BASE}/`) ? null : undefined),
  async start() {
    messengerStore.ensureLoaded().catch(() => {});
    // Taps on notifications are followed from the start of the app, not
    // from the first screen of the messenger that is shown.
    startPushBridge().catch(() => {});
  },
  stop() {
    messengerStore.stop();
  },
  // A tapped notification that started the app leads further than the default app.
  async resume() {
    const tapped = await launchRoute();
    if (!tapped) return false;
    await followRoute(tapped, true);
    return true;
  },
};

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The messenger as the desktop shell sees it (platform-spec 11.1).

import { get } from 'svelte/store';
import type { ModuleDef } from '$lib/core/module';
import { t } from '$lib/core/i18n';
import { common, navItem } from './common';
import { messengerStore } from '../store.svelte';
import { startDesktopNotices } from '../push/desktop';
import DesktopNotices from '../push/DesktopNotices.svelte';
import { messengerPalette } from '../palette.svelte';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

let stopNotices: (() => void) | null = null;

export const module: ModuleDef = {
  ...common,
  nav: [navItem],
  // The module's switch is in the Modules section of the core (section 12).
  settings: [],
  overlays: [{ id: 'notices', component: DesktopNotices, place: 'window' }],
  // New chat, new group, contacts, settings and every chat by name in the palette.
  palette: messengerPalette,
  // "{n} unread" in the tray's tooltip; the backend fills in the count.
  tray: () => ({ unread: get(t)('tray_unread') }),
  async start() {
    messengerStore.ensureLoaded().catch(() => {});
    // The messenger's notifications: words for the system, clicks back to a chat.
    if (stopNotices || !isTauri) return;
    stopNotices = await startDesktopNotices().catch(() => () => {});
  },
  stop() {
    stopNotices?.();
    stopNotices = null;
    messengerStore.stop();
  },
};

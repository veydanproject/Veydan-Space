// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The messenger as the desktop shell sees it (platform-spec 11.1).

import { get } from 'svelte/store';
import type { ModuleDef } from '$lib/core/module';
import { t } from '$lib/core/i18n';
import { accountOf, common, navItem } from './common';
import { BASE } from '../mobile/routes';
import { messengerStore } from '../store.svelte';
import { startDesktopNotices } from '../push/desktop';
import DesktopNotices from '../push/DesktopNotices.svelte';
import CallOverlay from '../calls/CallOverlay.svelte';
import IncomingLink from '../incoming/IncomingLink.svelte';
import ArchiveGate from '../archive/ArchiveGate.svelte';
import ServerAcceptDialog from '../servers/ServerAcceptDialog.svelte';
import ServerInfoDialog from '../servers/ServerInfoDialog.svelte';
import PrivateInviteDialog from '../servers/PrivateInviteDialog.svelte';
import { chatSettingsSections } from '../pages/chatSettingsSections';
import { messengerPalette } from '../palette.svelte';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

let stopNotices: (() => void) | null = null;

export const module: ModuleDef = {
  ...common,
  nav: [navItem],
  // Its panes go 12 from the window's sides, like the columns of Notes.
  fullWidth: [BASE],
  // The settings are a pane of the messenger's page here.
  account: accountOf(`${BASE}?view=settings`),
  // The module's switch is in the Modules section of the core (section 12).
  // The group «Чаты» of the core's Settings: the tabs of the chat's settings.
  settings: chatSettingsSections(),
  // The card of a ringing call and the panel of the call under way float over any page.
  overlays: [
    { id: 'notices', component: DesktopNotices, place: 'window' },
    { id: 'calls', component: CallOverlay, place: 'window' },
    // A link clicked outside the app: its card, once the app is unlocked.
    { id: 'incoming-link', component: IncomingLink, place: 'window' },
    // The first screen after a restore or a linking, while the newest months of the archive come back.
    { id: 'archive-gate', component: ArchiveGate, place: 'window' },
    // The questions of a server, one each for the window: a chat asks them, and so do the core's Settings,
    // where the messenger's page is not mounted (ROO-1075).
    { id: 'server-accept', component: ServerAcceptDialog, place: 'window' },
    { id: 'server-info', component: ServerInfoDialog, place: 'window' },
    { id: 'private-invite', component: PrivateInviteDialog, place: 'window' },
  ],
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

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The messenger as the mobile shell sees it (platform-spec 11.1).

import type { ModuleDef, NavItem } from '$lib/core/module';
import { accountOf, common, navItem } from './common';
import { messengerStore } from '../store.svelte';
import { chatStore } from '../chats/chatStore.svelte';
import { callLog } from '../calls/callLog.svelte';
import { followRoute, launchRoute, startPushBridge } from '../push/bridge';
import { BASE, callsHref, groupsHref, sectionHref } from '../mobile/routes';
import { chatSections } from '../seams/sections.svelte';
import NoticeBanner from '../push/NoticeBanner.svelte';
import CallWatcher from '../calls/CallWatcher.svelte';

/**
 * The bar of the messenger's tabs: «Чаты», «Группы», the sections modules
 * give («Каналы»), «Звонки», then the Settings of the app. Space puts its
 * apps' button in the middle of it.
 */
export function tabs(): NavItem[] {
  return [
    { id: 'chats', title: 'msg_nav_chats', icon: 'message-circle', href: BASE, badge: () => chatStore.unreadOf('dm') },
    { id: 'groups', title: 'msg_nav_groups', icon: 'users', href: groupsHref(), badge: () => chatStore.unreadOf('group') },
    ...chatSections.hosts.map((h): NavItem => ({ id: h.id, title: h.title, icon: h.icon, href: sectionHref(h.id), badge: () => h.unread() })),
    { id: 'calls', title: 'msg_nav_calls', icon: 'phone', href: callsHref(), badge: () => callLog.missed },
    { id: 'settings', title: 'app_settings', icon: 'settings', href: '/settings' },
  ];
}

const TABS = new Set([BASE, groupsHref(), callsHref(), `${BASE}/section`]);

export const module: ModuleDef = {
  ...common,
  nav: [navItem],
  // The settings are a screen of their own here.
  account: accountOf(`${BASE}/settings`),
  // The module's switch is in the Modules section of the core (section 12).
  settings: [],
  // A call opens its page from any screen, and stays a tap away when left.
  overlays: [
    { id: 'notices', component: NoticeBanner, place: 'window' },
    { id: 'calls', component: CallWatcher, place: 'window' },
  ],
  // The tabs keep the bar of the tabs; the chat, contacts and settings screens use the whole height.
  bar: (url) => (TABS.has(url.pathname.replace(/\/$/, '')) ? tabs() : url.pathname.startsWith(`${BASE}/`) ? null : undefined),
  // The bar of the tabs has no apps button: the header of each tab carries it.
  hubInHeader: (url) => TABS.has(url.pathname.replace(/\/$/, '')),
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

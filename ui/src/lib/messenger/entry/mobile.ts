// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The messenger as the mobile shell sees it (platform-spec 11.1). A wide
// window shows a chat, a group, a channel beside its list (platform-spec 11.7).

import type { ModuleDef, NavItem, SplitDef } from '$lib/core/module';
import { accountOf, common, navItem } from './common';
import { messengerStore } from '../store.svelte';
import { chatStore } from '../chats/chatStore.svelte';
import { callLog } from '../calls/callLog.svelte';
import { followRoute, launchRoute, startPushBridge } from '../push/bridge';
import { BASE, callsHref, contactsHref, groupsHref, sectionHref } from '../mobile/routes';
import MobileChats from '../mobile/MobileChats.svelte';
import { chatSections } from '../seams/sections.svelte';
import { isTauriHost } from '../api';
import { provideMessengerAttach } from '../attach/builtin';
import ChatSettingsRows from '../pages/ChatSettingsRows.svelte';
import ProfilePane from '../pages/ProfilePane.svelte';
import NoticeBanner from '../push/NoticeBanner.svelte';
import CallWatcher from '../calls/CallWatcher.svelte';
import IncomingLink from '../incoming/IncomingLink.svelte';
import ShareIn from '../share/ShareIn.svelte';
import ArchiveGate from '../archive/ArchiveGate.svelte';
import MobileDialogs from '../mobile/MobileDialogs.svelte';

/**
 * The bar of the messenger's tabs: «Чаты», «Группы», the sections modules
 * give («Каналы»), «Контакты», «Звонки». Space puts its apps' button in the middle of
 * it; the settings are behind the avatar in each tab's header.
 */
export function tabs(): NavItem[] {
  return [
    { id: 'chats', title: 'msg_nav_chats', icon: 'message-circle', href: BASE, badge: () => chatStore.unreadOf('dm') },
    { id: 'groups', title: 'msg_nav_groups', icon: 'users', href: groupsHref(), badge: () => chatStore.unreadOf('group') },
    ...chatSections.hosts.map((h): NavItem => ({ id: h.id, title: h.title, icon: h.icon, href: sectionHref(h.id), badge: () => h.unread() })),
    { id: 'contacts', title: 'msg_contacts_title', icon: 'book-user', href: contactsHref() },
    { id: 'calls', title: 'msg_nav_calls', icon: 'phone', href: callsHref(), badge: () => callLog.missed, danger: true },
  ];
}

const TABS = new Set([BASE, groupsHref(), callsHref(), contactsHref(), `${BASE}/section`]);

/** The lists a chat stands beside: the chats between two, the groups. One object each: the shell keeps the list mounted while it is the same. */
const DM_LIST: SplitDef = { home: BASE, list: MobileChats, props: { kind: 'dm' }, empty: { icon: 'message-circle', title: 'msg_welcome_title', text: 'msg_welcome_text' } };
const GROUP_LIST: SplitDef = { home: groupsHref(), list: MobileChats, props: { kind: 'group' }, empty: { icon: 'users', title: 'msg_groups_welcome_title', text: 'msg_groups_welcome_text' } };
const sectionLists = new Map<string, SplitDef>();
function sectionList(id: string): SplitDef {
  let def = sectionLists.get(id);
  if (!def) {
    const host = chatSections.get(id);
    const empty = host ? { icon: host.icon, title: host.welcomeTitle, text: host.welcomeText } : undefined;
    def = { home: sectionHref(id), list: MobileChats, props: { kind: id }, empty };
    sectionLists.set(id, def);
  }
  return def;
}

/**
 * The list a path stands beside: a chat beside the chats or the groups (by
 * its id, so a deleted chat still has its list), a row of a section beside
 * its section. The contacts, the calls, the settings: a screen as on a phone.
 */
export function splitOf(url: URL): SplitDef | undefined {
  const path = url.pathname.replace(/\/$/, '');
  const id = url.searchParams.get('id') ?? '';
  if (path === BASE) return DM_LIST;
  if (path === groupsHref()) return GROUP_LIST;
  if (path === `${BASE}/section`) return chatSections.get(id) ? sectionList(id) : undefined;
  if (path === `${BASE}/chat`) return id.startsWith('group:') ? GROUP_LIST : DM_LIST;
  if (path === `${BASE}/item`) {
    const host = chatSections.ofChat(id);
    return host ? sectionList(host.id) : undefined;
  }
  return undefined;
}

/** The messenger's items of the sheet of the paperclip, given at the start and taken back at the stop. */
let stopAttach: (() => void) | null = null;

export const module: ModuleDef = {
  ...common,
  nav: [navItem],
  // The settings are a screen of their own here.
  account: accountOf(`${BASE}/settings`),
  // The module's switch is in the Modules section of the core (section 12).
  // «Чаты» on the settings page, after the account: a row per tab of the chat's settings.
  settings: [
    { id: 'chats', title: 'msg_nav_chats', group: 'msg_nav_chats', order: 2, icon: 'message-circle', after: 'account', component: ChatSettingsRows },
    // Chat alone: the avatar opens the settings on «Профиль и ключ», «Настройки» beside it (ROO-558). Space opens the
    // same pane on its own screen (mobile/MobileProfile) and shows no such tab.
    { id: 'profile', title: 'msg_settings_tab_profile', group: 'msg_nav_chats', order: 0, icon: 'key', into: 'tab', component: ProfilePane, visible: () => !!messengerStore.identity },
  ],
  // A call opens its page from any screen, and stays a tap away when left.
  overlays: [
    { id: 'notices', component: NoticeBanner, place: 'window' },
    { id: 'calls', component: CallWatcher, place: 'window' },
    // A link clicked outside the app: its card, once the app is unlocked.
    { id: 'incoming-link', component: IncomingLink, place: 'window' },
    // What another app shared into Veydan: the chooser of a chat, once the app is unlocked.
    { id: 'share-in', component: ShareIn, place: 'window' },
    // The first screen after a restore or a linking, while the newest months of the archive come back.
    { id: 'archive-gate', component: ArchiveGate, place: 'window' },
    // The report and the servers' dialogs, outside the frames: a wide window shows two, folding takes one away.
    { id: 'dialogs', component: MobileDialogs, place: 'window' },
  ],
  // The tabs keep the bar of the tabs; the chat and settings screens use the whole height.
  bar: (url) => (TABS.has(url.pathname.replace(/\/$/, '')) ? tabs() : url.pathname.startsWith(`${BASE}/`) ? null : undefined),
  split: splitOf,
  async start() {
    if (!stopAttach) {
      const stops = [provideMessengerAttach()];
      // The browser preview adds a test item to show that an item is one file (attach/previewItem.ts).
      if (import.meta.env.DEV && !isTauriHost) stops.push((await import('../attach/previewItem')).providePreviewAttach());
      stopAttach = () => stops.forEach((stop) => stop());
    }
    messengerStore.ensureLoaded().catch(() => {});
    // Taps on notifications are followed from the start of the app, not
    // from the first screen of the messenger that is shown.
    startPushBridge().catch(() => {});
  },
  stop() {
    stopAttach?.();
    stopAttach = null;
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

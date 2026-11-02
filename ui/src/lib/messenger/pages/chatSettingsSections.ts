// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The chat's settings as sections of the core's Settings: a group «Чаты» of the six tabs on a computer.

import type { SettingsSection } from '$lib/core/module';
import ChatSettingsTab from './ChatSettingsTab.svelte';
import { SETTINGS_ICONS, SETTINGS_TABS, settingsUi } from './settingsTab.svelte';

/** Right after the account (order 1 of the cloud), before the browser (10). */
export function chatSettingsSections(): SettingsSection[] {
  return SETTINGS_TABS.map((tab) => ({
    id: `chats-${tab}`,
    title: `msg_settings_tab_${tab}` as SettingsSection['title'],
    group: 'msg_nav_chats',
    order: 2,
    icon: SETTINGS_ICONS[tab],
    bare: true,
    component: ChatSettingsTab,
    props: { tab },
    onopen: () => { settingsUi.tab = tab; },
    current: () => settingsUi.tab === tab,
  }));
}

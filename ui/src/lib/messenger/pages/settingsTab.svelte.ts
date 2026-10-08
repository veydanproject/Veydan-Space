// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Opens the messenger's settings on one of their tabs from elsewhere in the
// module (a long press on a call button opens "Calls"). The tab is asked for
// here; SettingsView takes it and clears it, on the desk's page as on the
// phone's screen of settings. The module knows nothing of the platform
// (shared/phone.ts): the caller says whether it is on a phone screen.

import { goto } from '$app/navigation';
import { BASE } from '../mobile/routes';
import { paletteUi } from '../palette.svelte';

export type SettingsTab = 'profile' | 'network' | 'notifications' | 'privacy' | 'calls' | 'diagnostics';

export const SETTINGS_TABS: readonly SettingsTab[] = ['profile', 'network', 'notifications', 'privacy', 'calls', 'diagnostics'];

/** The tab the next SettingsView shows (or the one shown switches to). */
export const settingsRequest = $state<{ tab: SettingsTab | null }>({ tab: null });

/**
 * The settings, on `tab`: the phone's screen of them (`phone`, as
 * `onPhone()` says where the caller is set up), or the desk's pane.
 */
export function openSettings(tab: SettingsTab, phone: boolean): void {
  settingsRequest.tab = tab;
  if (phone) void goto(`${BASE}/settings`);
  else paletteUi.request = 'settings';
}

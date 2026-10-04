// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The pass module as the desktop shell sees it (platform-spec 11.1): the
// Pass page (passwords, TOTP, the generator; 11.7) in the navigation, and in
// Space the same three as drawers behind the top bar's buttons for quick
// access (in Pass the window is that page: no drawers, `quick`).

import { get } from 'svelte/store';
import type { ModuleDef } from '$lib/core/module';
import { t } from '$lib/core/i18n';
import { common } from './common';
import { withInline } from '$lib/pass/entities';
import { registerPassCommands } from '$lib/pass/commands';
import { passUi } from '$lib/pass/store/ui.svelte';
import { showPass } from '$lib/pass/desktop/show';
import { passwordStore } from '$lib/pass/store/passwords.svelte';
import { totpStore } from '$lib/pass/store/totp.svelte';
import PassOverlays from '$lib/pass/desktop/PassOverlays.svelte';
import PasswordCardInline from '$lib/pass/desktop/PasswordCardInline.svelte';
import TotpCardInline from '$lib/pass/desktop/TotpCardInline.svelte';
import ProfileTotpTab from '$lib/pass/desktop/ProfileTotpTab.svelte';
import ProfilePasswordsTab from '$lib/pass/desktop/ProfilePasswordsTab.svelte';
import WorkspaceTotpDrawer from '$lib/pass/desktop/WorkspaceTotpDrawer.svelte';
import WorkspacePasswordsDrawer from '$lib/pass/desktop/WorkspacePasswordsDrawer.svelte';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

let stopListening: (() => void) | null = null;

export const module: ModuleDef = {
  ...common,
  entities: withInline({ password: { inline: PasswordCardInline }, totp: { inline: TotpCardInline } }),
  nav: [{ id: 'passwords', title: 'pw_title', icon: 'lock', href: '/passwords' }],
  settings: [],
  commands: registerPassCommands,
  tray: () => ({ password_generator: get(t)('tray_password_generator') }),
  tools: [
    { id: 'totp', title: 'totp_title', icon: 'shield', quick: true, onclick: () => { passUi.totpOpen = !passUi.totpOpen; } },
    { id: 'passwords', title: 'pw_title', icon: 'lock', quick: true, onclick: () => { passUi.passwordsOpen = !passUi.passwordsOpen; } },
    { id: 'generator', title: 'pwgen_title', icon: 'key', quick: true, onclick: () => { passUi.generatorOpen = !passUi.generatorOpen; } },
  ],
  // Quick access from the rest of Space; in Pass the window is the Pass page itself.
  overlays: [{ id: 'drawers', component: PassOverlays, place: 'window', quick: true }],
  views: [
    { id: 'totp', scope: 'profile', as: 'tab', title: 'totp_label', icon: 'shield', order: 10, count: (id) => totpStore.countForProfile(id), component: ProfileTotpTab },
    { id: 'passwords', scope: 'profile', as: 'tab', title: 'pw_title', icon: 'lock', order: 20, count: (id) => passwordStore.countForProfile(id), component: ProfilePasswordsTab },
    { id: 'totp', scope: 'workspace', as: 'drawer', title: 'totp_title', label: 'totp_label', icon: 'shield', order: 10, component: WorkspaceTotpDrawer },
    { id: 'passwords', scope: 'workspace', as: 'drawer', title: 'pw_title', icon: 'lock', order: 20, component: WorkspacePasswordsDrawer },
  ],
  async start() {
    void totpStore.ensureLoaded();
    void passwordStore.ensureLoaded();
    if (stopListening || !isTauri) return;
    const { listen } = await import('@tauri-apps/api/event');
    stopListening = await listen('tray://open-pwgen', () => showPass('generator'));
  },
  stop() {
    stopListening?.();
    stopListening = null;
  },
};

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The ssh module as the desktop shell sees it (platform-spec 11.1).

import { get } from 'svelte/store';
import type { ModuleDef } from '$lib/core/module';
import { t } from '$lib/core/i18n';
import { common } from './common';
import SSHSessionBar from '$lib/ssh/components/SSHSessionBar.svelte';
import SshTerminals from '$lib/ssh/desktop/SshTerminals.svelte';
import ProfileSshTab from '$lib/ssh/desktop/ProfileSshTab.svelte';
import WorkspaceSshDrawer from '$lib/ssh/desktop/WorkspaceSshDrawer.svelte';

export const module: ModuleDef = {
  ...common,
  nav: [
    { id: 'terminal', title: 'nav_terminal', icon: 'terminal', href: '/terminal' },
    { id: 'files', title: 'nav_files', icon: 'folder', href: '/files' },
  ],
  settings: [],
  tray: () => ({ section_terminal: get(t)('nav_terminal'), section_files: get(t)('nav_files') }),
  overlays: [
    { id: 'terminals', component: SshTerminals, place: 'content' },
    { id: 'sessions', component: SSHSessionBar, place: 'bottom', order: 10 },
  ],
  views: [
    { id: 'ssh', scope: 'profile', as: 'tab', title: 'panel_tab_ssh', icon: 'terminal', order: 40, component: ProfileSshTab },
    { id: 'ssh', scope: 'workspace', as: 'drawer', title: 'ssh_title', label: 'ssh_bar_label', icon: 'terminal', order: 40, component: WorkspaceSshDrawer },
  ],
};

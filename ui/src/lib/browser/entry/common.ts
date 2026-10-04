// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the browser module is regardless of the shell (platform-spec 11.1).
// The module exists on the desktop only (1.2); there is no mobile entry.

import type { ModuleDef } from '$lib/core/module';
import { browserEntities } from '$lib/browser/entities';
import { workspacesStore } from '$lib/browser/store/workspaces.svelte';
import { profilesStore } from '$lib/browser/store/profiles.svelte';
import { proxiesStore } from '$lib/browser/store/proxies.svelte';
import { runningStore } from '$lib/browser/store/running.svelte';
import { api } from '$lib/browser/api';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

let stopListening: (() => void) | null = null;

export const common: Omit<ModuleDef, 'nav' | 'settings'> = {
  id: 'browser',
  title: 'nav_workspaces',
  icon: 'layers',
  routes: ['/', '/workspace', '/proxies'],
  entities: browserEntities,
  demo: true,
  // Browser profile files travel through the vault as large files.
  syncsFiles: true,
  // About names what the profiles are built on.
  about: [{ id: 'camoufox', title: 'settings_about_camoufox', url: 'https://camoufox.com/' }],
  reloaders: {
    workspace: () => workspacesStore.loaded ? workspacesStore.refresh() : Promise.resolve(),
    workspace_column: () => workspacesStore.loaded ? workspacesStore.refresh() : Promise.resolve(),
    profile: () => profilesStore.loaded ? profilesStore.refresh() : Promise.resolve(),
    proxy: () => proxiesStore.loaded ? proxiesStore.refresh() : Promise.resolve(),
  },
  async start() {
    if (stopListening) return;
    void profilesStore.ensureLoaded();
    if (!isTauri) return;
    const { listen } = await import('@tauri-apps/api/event');
    const stops = await Promise.all([
      runningStore.listen(),
      // The tray's profile entries, built by the backend from this module's data.
      listen<string>('tray://launch-profile', (e) => {
        api.profiles.launch(e.payload).catch((err) => console.error(err));
      }),
      listen<string>('tray://stop-profile', (e) => {
        api.profiles.stop(e.payload).catch((err) => console.error(err));
      }),
      listen('tray://stop-all', () => {
        runningStore.ids.forEach((id) => api.profiles.stop(id).catch(() => {}));
      }),
    ]);
    stopListening = () => stops.forEach((fn) => fn());
  },
  stop() {
    stopListening?.();
    stopListening = null;
  },
  // The bug report names the installed Camoufox.
  async report() {
    const status = await api.camoufox.status().catch(() => null);
    return status?.installed && status.version ? [`Camoufox: ${status.version}`] : [];
  },
};

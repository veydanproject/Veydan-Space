// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api, type LaunchingProfiles } from '$lib/browser/api';

/** Ids of the browser profiles running right now, as the backend reports them. */
class RunningStore {
  ids = $state<string[]>([]);
  /** Profiles being launched, from any window, button or the tray. */
  launching = $state<string[]>([]);
  /** Of those, the ones waiting for their files to sync. */
  syncing = $state<string[]>([]);

  async refresh() {
    try {
      this.ids = await api.profiles.runningIds();
      this.setLaunching(await api.profiles.launching());
    } catch {}
  }

  private setLaunching(p: LaunchingProfiles) {
    this.launching = p.launching;
    this.syncing = p.syncing;
  }

  /** Follows the backend's events for as long as the window lives; returns what stops it. */
  async listen(): Promise<() => void> {
    const { listen } = await import('@tauri-apps/api/event');
    const unlisten = await Promise.all([
      listen<{ running_ids: string[] }>('profiles://running-changed', (e) => {
        this.ids = e.payload.running_ids;
      }),
      listen<LaunchingProfiles>('profiles://launching-changed', (e) => this.setLaunching(e.payload)),
    ]);
    const onFocus = () => void this.refresh();
    window.addEventListener('focus', onFocus);
    await this.refresh();
    return () => {
      unlisten.forEach((fn) => fn());
      window.removeEventListener('focus', onFocus);
    };
  }
}

export const runningStore = new RunningStore();

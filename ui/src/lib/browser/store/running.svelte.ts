// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api } from '$lib/browser/api';

/** Ids of the browser profiles running right now, as the backend reports them. */
class RunningStore {
  ids = $state<string[]>([]);

  async refresh() {
    try {
      this.ids = await api.profiles.runningIds();
    } catch {}
  }

  /** Follows the backend's events for as long as the window lives; returns what stops it. */
  async listen(): Promise<() => void> {
    const { listen } = await import('@tauri-apps/api/event');
    const unlisten = await listen<{ running_ids: string[] }>('profiles://running-changed', (e) => {
      this.ids = e.payload.running_ids;
    });
    const onFocus = () => void this.refresh();
    window.addEventListener('focus', onFocus);
    await this.refresh();
    return () => {
      unlisten();
      window.removeEventListener('focus', onFocus);
    };
  }
}

export const runningStore = new RunningStore();

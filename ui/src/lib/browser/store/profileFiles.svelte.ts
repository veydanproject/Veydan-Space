// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api, type ProfileFilesStatus } from '$lib/browser/api';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** How often the upload of profile files is looked at while it runs. */
const SENDING_EVERY_MS = 1000;

/** Where the files of the profiles stand that are not out from here, or not here: for their badges. */
class ProfileFilesStore {
  byId = $state<Record<string, ProfileFilesStatus>>({});
  private _unlisten: (() => void)[] = [];
  private _listeners = 0;
  private _last = 0;

  async refresh() {
    if (!isTauri) return;
    try {
      const list = await api.profiles.filesStatus();
      this.byId = Object.fromEntries(list.map((s) => [s.profile_id, s]));
    } catch {}
  }

  /** Follows sync while a badge is shown; returns what stops it. */
  async listen(): Promise<() => void> {
    if (!isTauri) return () => {};
    this._listeners++;
    void this.refresh();
    if (this._listeners === 1) {
      const { listen } = await import('@tauri-apps/api/event');
      this._unlisten = await Promise.all([
        listen('sync://status', () => void this.refresh()),
        listen('profiles://running-changed', () => void this.refresh()),
        listen<{ phase?: string }>('sync://progress', (e) => {
          const now = Date.now();
          if (e.payload?.phase !== 'profiles_up' || now - this._last < SENDING_EVERY_MS) return;
          this._last = now;
          void this.refresh();
        }),
      ]);
    }
    return () => {
      this._listeners--;
      if (this._listeners <= 0) {
        this._unlisten.forEach((fn) => fn());
        this._unlisten = [];
      }
    };
  }
}

export const profileFilesStore = new ProfileFilesStore();

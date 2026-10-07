// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api } from '$lib/core/api';
import type { SyncProgress, SyncStatus } from '$lib/core/api';
import { formatError } from '$lib/core/utils';
import { isLocale, locale } from '$lib/core/i18n';
import { registry } from '$lib/core/registry';
import { get } from 'svelte/store';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** Follow the UI language another device saved. */
async function applyRemoteLocale() {
  const remote = await api.settings.getLocale();
  // A language this version does not have stays as it is here (platform-spec 9.5).
  if (isLocale(remote) && remote !== get(locale)) locale.set(remote);
}

/**
 * Store reloads per sync entity applied from another device: the core's own
 * (settings), and each module's from `ModuleDef.reloaders` through the registry.
 */
function reloadersFor(entity: string): (() => Promise<unknown>)[] {
  const own = entity === 'setting' ? [applyRemoteLocale] : [];
  return [...own, ...registry.reloadersFor(entity)];
}

/** Vault sync status shared by the Notes sync button and Settings. */
class SyncStore {
  status = $state<SyncStatus | null>(null);
  progress = $state<SyncProgress | null>(null);
  busy = $state(false);
  error = $state('');
  private _unlisten: (() => void) | null = null;
  private _unlistenProgress: (() => void) | null = null;
  private _listeners = 0;

  async refresh() {
    if (!isTauri) return;
    try {
      this.status = await api.sync.status();
      if (!this.status?.running) this.progress = null;
    } catch {}
  }

  /** Subscribe to backend status events; returns an unsubscribe fn. */
  async listen(): Promise<() => void> {
    if (!isTauri) return () => {};
    this._listeners++;
    void this.refresh();
    if (!this._unlisten) {
      const { listen } = await import('@tauri-apps/api/event');
      this._unlisten = await listen('sync://status', () => void this.refresh());
      this._unlistenProgress = await listen<SyncProgress>('sync://progress', (e) => {
        this.progress = e.payload;
      });
    }
    return () => {
      this._listeners--;
      if (this._listeners <= 0) {
        this._unlisten?.();
        this._unlisten = null;
        this._unlistenProgress?.();
        this._unlistenProgress = null;
      }
    };
  }

  /** Reload stores whose rows another device changed; returns an unsubscribe fn. */
  async listenDataChanged(): Promise<() => void> {
    if (!isTauri) return () => {};
    const { listen } = await import('@tauri-apps/api/event');
    return listen<string[]>('sync://data-changed', (e) => {
      for (const entity of e.payload) {
        for (const reload of reloadersFor(entity)) reload().catch(() => {});
      }
    });
  }

  /** Full vault cycle now; without a joined vault the caller's `fallback` runs instead (the notes' file reindex). */
  async runNow(fallback?: () => Promise<void>) {
    if (this.busy) return;
    this.busy = true;
    this.error = '';
    try {
      if (this.status?.joined) {
        this.status = await api.sync.runNow();
      } else {
        await fallback?.();
      }
    } catch (e) {
      this.error = formatError(e);
    } finally {
      this.busy = false;
      await this.refresh();
    }
  }
}

export const syncStore = new SyncStore();

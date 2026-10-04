// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api } from '$lib/core/api';
import type { Update } from '@tauri-apps/plugin-updater';

export type UpdaterStatus =
  | 'idle'
  | 'checking'
  | 'available'
  | 'downloading'
  | 'installing'
  | 'upToDate'
  | 'error';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

class UpdaterStore {
  status = $state<UpdaterStatus>('idle');
  /** Version offered by the update endpoint (when status === 'available') */
  version = $state('');
  /** Download progress 0-100 */
  progress = $state(0);
  error = $state('');
  bannerDismissed = $state(false);
  /** False on Linux deb/rpm installs — updater can't replace those in-place */
  supported = $state(true);
  private update: Update | null = null;
  private initialized = false;

  async init() {
    if (!isTauri || this.initialized) return;
    this.initialized = true;
    try {
      this.supported = await api.system.updateSupported();
    } catch {
      // Command missing (old backend) — keep the optimistic default.
    }
  }

  /**
   * Check the release endpoint for a newer version.
   * silent=true (startup): errors are swallowed, nothing changes on failure.
   * silent=false (settings button): errors surface via status='error'.
   */
  async check(silent = false) {
    if (!isTauri) return;
    if (this.status === 'checking' || this.status === 'downloading' || this.status === 'installing')
      return;
    await this.init();
    this.status = 'checking';
    this.error = '';
    try {
      const { check } = await import('@tauri-apps/plugin-updater');
      const update = await check();
      if (update) {
        this.update = update;
        this.version = update.version;
        this.status = 'available';
      } else {
        this.update = null;
        this.status = 'upToDate';
      }
    } catch (e) {
      this.update = null;
      if (silent) {
        this.status = 'idle';
      } else {
        // The plugin's text is English and technical: the card shows its own
        // sentence (settings_update_failed), this one is for the console.
        console.error('update check failed', e);
        this.error = String(e);
        this.status = 'error';
      }
    }
  }

  async install() {
    if (!isTauri || !this.update || !this.supported) return;
    if (this.status === 'downloading' || this.status === 'installing') return;
    this.status = 'downloading';
    this.progress = 0;
    this.error = '';
    try {
      let total = 0;
      let received = 0;
      await this.update.downloadAndInstall((event) => {
        switch (event.event) {
          case 'Started':
            total = event.data.contentLength ?? 0;
            break;
          case 'Progress':
            received += event.data.chunkLength;
            if (total > 0) this.progress = Math.min(100, Math.round((received / total) * 100));
            break;
          case 'Finished':
            this.progress = 100;
            break;
        }
      });
      this.status = 'installing';
      // On Windows the installer exits the app itself; elsewhere we relaunch.
      const { relaunch } = await import('@tauri-apps/plugin-process');
      await relaunch();
    } catch (e) {
      this.error = String(e);
      this.status = 'error';
    }
  }

  dismiss() {
    this.bannerDismissed = true;
  }
}

export const updaterStore = new UpdaterStore();

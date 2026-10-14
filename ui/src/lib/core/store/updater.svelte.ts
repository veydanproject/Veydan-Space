// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api } from '$lib/core/api';

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
  /** The last check found a newer version. */
  private offered = false;
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
   * Check the release endpoint for a newer version (the shell's `updater_check`: never an
   * older one).
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
      const version = await api.updater.check();
      if (version) {
        this.offered = true;
        this.version = version;
        this.status = 'available';
      } else {
        this.offered = false;
        this.status = 'upToDate';
      }
    } catch (e) {
      this.offered = false;
      if (silent) {
        this.status = 'idle';
      } else {
        // The updater's text is English and technical: the card shows its own
        // sentence (settings_update_failed), this one is for the console.
        console.error('update check failed', e);
        this.error = String(e);
        this.status = 'error';
      }
    }
  }

  async install() {
    if (!isTauri || !this.offered || !this.supported) return;
    if (this.status === 'downloading' || this.status === 'installing') return;
    this.status = 'downloading';
    this.progress = 0;
    this.error = '';
    try {
      let received = 0;
      await api.updater.install((event) => {
        switch (event.event) {
          case 'chunk':
            received += event.len;
            if (event.total) this.progress = Math.min(100, Math.round((received / event.total) * 100));
            break;
          case 'downloaded':
            this.progress = 100;
            break;
        }
      });
      this.status = 'installing';
      // On Windows the installer exits the app itself (the modules stopped
      // first); elsewhere the shell stops the modules and restarts.
      await api.updater.relaunch();
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

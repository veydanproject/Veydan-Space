// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The check for a new version on a phone, where Tauri has no updater
// (internal/platform-spec.md 14.3). The shell reads the product's latest.json
// (`update_check`); this store keeps the answer for Settings. A computer
// uses the updater store instead.

import { api } from '$lib/core/api';
import { formatError } from '$lib/core/utils';
import type { UpdateCheck } from '$lib/core/types';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export type UpdateCheckStatus = 'idle' | 'checking' | 'available' | 'upToDate' | 'error';

type Check = (manual: boolean) => Promise<UpdateCheck>;

export class UpdateCheckStore {
  status = $state<UpdateCheckStatus>('idle');
  /** The version the product's latest.json offers. */
  latest = $state('');
  error = $state('');
  private started = false;

  private readonly ask: Check;

  constructor(ask: Check = (manual) => api.update.check(manual)) {
    this.ask = ask;
  }

  private take(answer: UpdateCheck, asked: boolean) {
    this.latest = answer.latest ?? '';
    // The user's check always asked the network; the start's may have skipped it.
    this.status = answer.available ? 'available' : answer.latest || asked ? 'upToDate' : 'idle';
  }

  /** The check of the start, once per run of the app; the shell decides
   *  whether it asks the network (once a day, not on a metered network). */
  async auto() {
    if (this.started) return;
    this.started = true;
    try {
      this.take(await this.ask(false), false);
    } catch {
      // Silent: the user did not ask.
    }
  }

  /** The user's check: always asks the network, reports a failure. */
  async check() {
    if (this.status === 'checking') return;
    this.status = 'checking';
    this.error = '';
    try {
      this.take(await this.ask(true), true);
    } catch (e) {
      // The page shows a translated sentence; the technical detail goes to the log.
      this.error = formatError(e);
      console.error('update check failed:', this.error);
      this.status = 'error';
    }
  }

  /** The release page of the new version, in the system browser. */
  async open() {
    try {
      await api.update.open();
    } catch (e) {
      this.error = formatError(e);
    }
  }
}

export const updateCheck = new UpdateCheckStore();

/** Starts the check of the start a little after the app came up. */
export function startUpdateCheck(delayMs = 5000): () => void {
  if (!isTauri) return () => {};
  const timer = setTimeout(() => void updateCheck.auto(), delayMs);
  return () => clearTimeout(timer);
}

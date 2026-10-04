// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The generator's history (the table `password_history`), shared by the
// desktop generator (drawer and pane) and the phone's generator screen. The
// switch and the limit are the generator's settings (`pwSettings`); the
// passwords stay in the local database of the device.

import { api } from '$lib/pass/api';
import { ask } from '$lib/core/ui/confirm.svelte';
import type { PwEntry } from '$lib/pass/password-gen';

class PwgenHistoryStore {
  /** Newest first, as the backend lists them. */
  entries = $state<PwEntry[]>([]);
  /** Whether `entries` reflects the database (a screen shows nothing before). */
  loaded = $state(false);

  /**
   * Counts the times the passwords left memory. A list read before a
   * `forget()` and answered after it is dropped: the lock that unmounted
   * the generators must not find them in memory again.
   */
  private epoch = 0;

  async load() {
    const epoch = this.epoch;
    try {
      const entries = await api.pwgen.list();
      if (epoch !== this.epoch) return;
      this.entries = entries;
      this.loaded = true;
    } catch {}
  }

  /** Save a generated password, keep the newest `limit` (null: all), list again. */
  async record(password: string, limit: number | null) {
    const epoch = this.epoch;
    try {
      await api.pwgen.add(password);
      if (limit !== null) await api.pwgen.trim(limit);
      // Forgotten meanwhile: the password is saved, the list is not read back.
      if (epoch === this.epoch) await this.load();
    } catch {}
  }

  async clear() {
    try {
      await api.pwgen.clear();
      this.entries = [];
    } catch {}
  }

  /**
   * Clear after the app's question — the desktop dialog, the phone's bottom
   * sheet (`ask()`) — with the same words on both. True when the user said
   * yes; nothing is deleted otherwise.
   */
  async confirmClear(tr: (key: 'pwgen_history_clear_title' | 'pwgen_history_clear_message' | 'pwgen_btn_clear') => string): Promise<boolean> {
    const yes = await ask({
      title: tr('pwgen_history_clear_title'),
      message: tr('pwgen_history_clear_message'),
      confirmLabel: tr('pwgen_btn_clear'),
    });
    if (yes) await this.clear();
    return yes;
  }

  /** Drop the passwords from memory (the database keeps them). */
  forget() {
    this.epoch++;
    this.entries = [];
    this.loaded = false;
  }

  private holders = 0;

  /**
   * A mounted generator holds the history; the returned function lets go.
   * When the last one goes — the lock unmounts them all — the passwords
   * leave memory, as they did when each generator kept its own list. One
   * generator closing (Space's pane while its drawer stays) keeps them.
   */
  hold(): () => void {
    this.holders++;
    let held = true;
    return () => {
      if (!held) return;
      held = false;
      this.holders--;
      if (this.holders === 0) this.forget();
    };
  }
}

export const pwgenHistory = new PwgenHistoryStore();

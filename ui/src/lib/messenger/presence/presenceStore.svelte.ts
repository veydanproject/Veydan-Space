// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// When approved contacts are online, as the runtime last heard it. "Online"
// is a deadline, not a flag: the clock here moves every 10 s, so a contact
// whose heartbeat stopped turns into "last seen" without a word from the
// runtime. Empty while the presence switch is off.

import { t, type TranslationKey } from '$lib/core/i18n';
import { messengerApi, type MessengerUiEvent } from '../api';
import { presenceLabel } from './labels';

const TICK_MS = 10_000;

interface Seen {
  seen_at: number;
  online_until: number;
}

const nowSecs = () => Math.floor(Date.now() / 1000);

class PresenceStore {
  /** Contact's key (hex) → when last seen and until when online. */
  map = $state<Record<string, Seen>>({});
  /** Unix seconds; moves every 10 s while started. */
  now = $state(nowSecs());
  private tr = $state<(key: TranslationKey, vars?: Record<string, string>) => string>((k) => k);
  private timer: ReturnType<typeof setInterval> | null = null;
  /** The presence switch (`setSharing`); on until the settings say otherwise. */
  private sharing = true;

  constructor() {
    // The labels follow a change of language at once, not at the next tick.
    t.subscribe((fn) => (this.tr = fn));
  }

  start() {
    if (this.timer) return;
    this.now = nowSecs();
    this.timer = setInterval(() => (this.now = nowSecs()), TICK_MS);
  }

  stop() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  async load() {
    if (!this.sharing) {
      this.map = {};
      return;
    }
    try {
      const list = await messengerApi.presence.list();
      const map: Record<string, Seen> = {};
      for (const p of Array.isArray(list) ? list : []) map[p.peer] = { seen_at: p.seen_at, online_until: p.online_until };
      this.map = map;
      this.now = nowSecs();
    } catch {
      this.map = {};
    }
  }

  /**
   * The presence switch as the privacy settings last said. Off, nobody is
   * shown, whatever event comes late; on again, the list is read anew.
   */
  setSharing(on: boolean) {
    if (on === this.sharing) return;
    this.sharing = on;
    if (on) this.load().catch(() => {});
    else this.map = {};
  }

  /** Runtime event → state. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    switch (ev.name) {
      case 'presence.updated': {
        if (!this.sharing) return;
        const p = ev.payload as { peer?: string; seen_at?: number; online_until?: number } | null;
        if (!p?.peer || typeof p.seen_at !== 'number' || typeof p.online_until !== 'number') return;
        // Somebody not on the list (a contact just removed, blocked, or not
        // yet approved here): the runtime's list says whether to show them.
        if (!(p.peer in this.map)) {
          this.load().catch(() => {});
          return;
        }
        this.map = { ...this.map, [p.peer]: { seen_at: p.seen_at, online_until: p.online_until } };
        this.now = nowSecs();
        return;
      }
      // A key came or went, a contact was removed or blocked, or the key or
      // the switch moved on another device: who can be seen changed.
      case 'presence.keys_changed':
      case 'presence.epoch_changed':
      case 'dm.relationship':
        this.load().catch(() => {});
        return;
    }
  }

  /** `'online'`, the time last seen, or `null` when nothing is known. */
  status(peer: string | null | undefined): 'online' | number | null {
    const s = peer ? this.map[peer] : undefined;
    if (!s) return null;
    return this.now < s.online_until ? 'online' : s.seen_at;
  }

  /** The line under the name, or `null` when nothing is known. */
  label(peer: string | null | undefined): string | null {
    const s = peer ? this.map[peer] : undefined;
    if (!s) return null;
    return presenceLabel(s.seen_at, s.online_until, this.now, this.tr);
  }

  reset() {
    this.stop();
    this.map = {};
    this.sharing = true;
  }
}

export const presenceStore = new PresenceStore();

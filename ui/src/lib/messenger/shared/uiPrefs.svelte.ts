// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// How wide I made the panes of the messenger on a computer: the runtime
// keeps the map and sends it to my other computers (`own.ui`); a change
// made on another one comes back as `ui_prefs.updated {w}` and an open
// window takes it at once. Keys: `channels.side` (the column of a channel),
// `chats.side` (the info panel of a direct chat and of a group). Where
// there is no runtime the panes keep their defaults.

import { messengerApi, type MessengerUiEvent } from '../api';

class UiPrefsStore {
  /** Key → px; empty until loaded. */
  widths = $state<Record<string, number>>({});

  async load() {
    try {
      const map = await messengerApi.uiPrefs.get();
      if (map && typeof map === 'object') this.widths = { ...map };
    } catch {
      // No runtime yet: the defaults stand.
    }
  }

  /** The width kept for `key`, or `null` for the pane's default. */
  width(key: string): number | null {
    const px = this.widths[key];
    return typeof px === 'number' && Number.isFinite(px) ? px : null;
  }

  /**
   * The pane `key` was made `px` wide (`null`: back to its default). It
   * holds here at once; the runtime keeps it and tells my other computers.
   */
  set(key: string, px: number | null) {
    const next = { ...this.widths };
    if (px == null) delete next[key];
    else next[key] = Math.round(px);
    this.widths = next;
    // Refused (no runtime, out of bounds): what the runtime keeps comes back.
    messengerApi.uiPrefs.set(key, px == null ? null : Math.round(px)).catch(() => this.load());
  }

  /** Runtime event → state. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    if (ev.name !== 'ui_prefs.updated') return;
    const w = (ev.payload as { w?: Record<string, number> } | null)?.w;
    if (w && typeof w === 'object') this.widths = { ...w };
  }

  reset() {
    this.widths = {};
  }
}

export const uiPrefs = new UiPrefsStore();

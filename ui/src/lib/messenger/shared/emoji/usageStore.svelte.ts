// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The emoji I used last and use most (ROO-297): the seven used last, the
// newest first, then the most used. The runtime counts them and keeps the
// map the same on every device of mine. Reactions and the composer's picker both
// start from this list. The local recents of `data.ts` stand in until the
// runtime has answered, and where there is no runtime at all.

import { messengerApi, type MessengerUiEvent } from '../../api';
import { loadRecent } from './data';

const TOP = 24;

class UsageStore {
  /** The used last first, then the most used; empty until loaded. */
  popular = $state<string[]>([]);

  async load() {
    try {
      const top = await messengerApi.emoji.top(TOP);
      this.popular = Array.isArray(top) ? top : loadRecent();
    } catch {
      this.popular = loadRecent();
    }
  }

  /** I picked this emoji in the composer: it is first at once, the runtime counts it. */
  used(emoji: string) {
    this.first(emoji);
    messengerApi.emoji.used(emoji).catch(() => {});
  }

  /** I put this emoji as a reaction: first at once; the runtime counted it, its order is read. */
  reacted(emoji: string) {
    this.first(emoji);
    this.load().catch(() => {});
  }

  private first(emoji: string) {
    this.popular = [emoji, ...this.popular.filter((e) => e !== emoji)].slice(0, TOP);
  }

  /** Runtime event → state. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    // Another device of mine sent its counts.
    if (ev.name === 'emoji.updated') this.load().catch(() => {});
  }

  reset() {
    this.popular = [];
  }
}

export const usageStore = new UsageStore();

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The requests of strangers: chats whose one text waits for my answer, kept
// apart from the chats until I accept. The runtime holds them; its events
// (`dm.request`, a changed list of chats, the end of a history sync) are
// only a reason to read them again, never the state itself, so a missed
// event costs nothing but a late count.

import { messengerApi, type MessengerUiEvent, type RequestAction, type RequestView } from '../api';
import { chatStore } from '../chats/chatStore.svelte';

const PAGE = 50;
/** Events come in bursts (a history sync, a sweep of old requests): one read for each burst. */
const RELOAD_MS = 200;

/** The module store imports this one: reached late, as the chat store does. */
function refreshContacts() {
  import('../store.svelte').then((m) => m.messengerStore.refreshContacts()).catch(() => {});
}

/** What a request is called: the name the runtime found, else a short form of the key. */
export function requestName(r: Pick<RequestView, 'name' | 'peer'>): string {
  const name = r.name.trim();
  if (name) return name;
  return r.peer.length > 12 ? `${r.peer.slice(0, 6)}…${r.peer.slice(-4)}` : r.peer;
}

/** The line of a request in the list: its text up to the first line break. */
export function firstLine(text: string): string {
  return text.trim().split('\n', 1)[0].trim();
}

class RequestStore {
  /** How many requests wait, as the runtime counts them: the row above the chats. */
  count = $state(0);
  /** Newest first, as many pages as were read. */
  items = $state<RequestView[]>([]);
  /** Where the next page starts; `null` at the end. */
  nextBefore = $state<number | null>(null);
  loaded = $state(false);
  loadingMore = $state(false);
  /** The request shown beside the list on the desktop, by the stranger's key. */
  selected = $state<string | null>(null);
  /** The stranger whose request is being answered: its buttons wait. */
  acting = $state<string | null>(null);
  private _timer: ReturnType<typeof setTimeout> | null = null;
  /** Bumped by `reset`: an answer to a read started before it is of another session. */
  private _epoch = 0;

  get current(): RequestView | null {
    return this.items.find((r) => r.peer === this.selected) ?? null;
  }

  find(peer: string): RequestView | null {
    return this.items.find((r) => r.peer === peer) ?? null;
  }

  /** The first pages again, as many as were shown: a reload does not shorten the list under the user. */
  load = async () => {
    const epoch = this._epoch;
    const page = await messengerApi.moderation.requests(undefined, Math.max(PAGE, this.items.length));
    if (epoch !== this._epoch) return;
    this.items = page.items;
    this.count = page.total;
    this.nextBefore = page.next_before ?? null;
    this.loaded = true;
  };

  loadMore = async () => {
    const before = this.nextBefore;
    if (before === null || this.loadingMore) return;
    const epoch = this._epoch;
    this.loadingMore = true;
    try {
      const page = await messengerApi.moderation.requests(before, PAGE);
      if (epoch !== this._epoch) return;
      const known = new Set(this.items.map((r) => r.peer));
      this.items = [...this.items, ...page.items.filter((r) => !known.has(r.peer))];
      this.count = page.total;
      this.nextBefore = page.next_before ?? null;
    } finally {
      this.loadingMore = false;
    }
  };

  /**
   * Answers a request. Gone from the list at once; the chats are read
   * again (an accepted one joins them, a blocked one is shown as blocked),
   * and so is the book: accepting adds the person to it. Refusals throw:
   * the view words them.
   */
  async act(peer: string, action: RequestAction): Promise<void> {
    this.acting = peer;
    try {
      await messengerApi.moderation.requestAct(peer, action);
    } finally {
      this.acting = null;
    }
    const had = this.items.some((r) => r.peer === peer);
    this.items = this.items.filter((r) => r.peer !== peer);
    if (had) this.count = Math.max(0, this.count - 1);
    if (this.selected === peer) this.selected = null;
    await chatStore.loadChats().catch(() => {});
    refreshContacts();
    this.scheduleReload();
  }

  scheduleReload() {
    if (this._timer) return;
    this._timer = setTimeout(() => {
      this._timer = null;
      this.load().catch(() => {});
    }, RELOAD_MS);
  }

  /** Runtime event → a read again. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    switch (ev.name) {
      case 'dm.request': {
        // The count the event carries is shown at once; the list follows.
        const pending = (ev.payload as { pending?: unknown } | null)?.pending;
        if (typeof pending === 'number') this.count = pending;
        this.scheduleReload();
        break;
      }
      // Another device of mine answered, the list of chats moved, history came in.
      case 'dm.relationship':
      case 'chats.updated':
      case 'history.synced':
        this.scheduleReload();
        break;
    }
  }

  reset() {
    this._epoch++;
    if (this._timer) clearTimeout(this._timer);
    this._timer = null;
    this.count = 0;
    this.items = [];
    this.nextBefore = null;
    this.loaded = false;
    this.selected = null;
    this.acting = null;
    requestsUi.shown = false;
  }
}

export const requestStore = new RequestStore();

/**
 * Whether the rail of the desktop page shows the requests in place of the
 * chats. A request and an open chat are never shown together: opening the
 * requests closes the chat, and the page closes the requests when a chat
 * opens.
 */
class RequestsUi {
  shown = $state(false);

  /** The list reads the requests as it mounts. */
  open() {
    chatStore.close();
    this.shown = true;
  }

  close() {
    this.shown = false;
    requestStore.selected = null;
  }
}

export const requestsUi = new RequestsUi();

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What each chat has shared: how many per section, and the pages read so
// far. Read when a panel shows it, read again when the chat changes.

import { messengerApi, type MessengerMessage, type MessengerUiEvent, type SharedCounts, type SharedSection } from '../../api';

/** One page of a section. */
export const PAGE = 60;
/** Changes that come in a burst are read once. */
const SETTLE_MS = 400;

export interface SectionPage {
  messages: MessengerMessage[];
  /** Older ones may exist. */
  more: boolean;
  loading: boolean;
}

interface ChatShared {
  counts: SharedCounts | null;
  sections: Partial<Record<SharedSection, SectionPage>>;
}

const EMPTY: ChatShared = { counts: null, sections: {} };

class SharedStore {
  private chats = $state<Record<string, ChatShared>>({});
  /** Chats a panel shows now: only these are read again on a change. */
  private watched = new Map<string, number>();
  private timers = new Map<string, ReturnType<typeof setTimeout>>();

  counts(chatId: string): SharedCounts | null {
    return this.chats[chatId]?.counts ?? null;
  }

  page(chatId: string, section: SharedSection): SectionPage | null {
    return this.chats[chatId]?.sections[section] ?? null;
  }

  /** A panel shows the chat; the returned function says it stopped. */
  watch(chatId: string): () => void {
    this.watched.set(chatId, (this.watched.get(chatId) ?? 0) + 1);
    this.loadCounts(chatId).catch(() => {});
    return () => {
      const n = (this.watched.get(chatId) ?? 1) - 1;
      if (n > 0) this.watched.set(chatId, n);
      else this.watched.delete(chatId);
    };
  }

  private patch(chatId: string, fn: (c: ChatShared) => ChatShared) {
    this.chats = { ...this.chats, [chatId]: fn(this.chats[chatId] ?? EMPTY) };
  }

  private setPage(chatId: string, section: SharedSection, page: SectionPage) {
    this.patch(chatId, (c) => ({ ...c, sections: { ...c.sections, [section]: page } }));
  }

  async loadCounts(chatId: string) {
    const counts = await messengerApi.chats.sharedCounts(chatId);
    this.patch(chatId, (c) => ({ ...c, counts }));
  }

  /** The newest page of a section, read anew. */
  async open(chatId: string, section: SharedSection) {
    const had = this.page(chatId, section);
    this.setPage(chatId, section, { messages: had?.messages ?? [], more: had?.more ?? false, loading: true });
    try {
      const messages = await messengerApi.chats.shared(chatId, section, undefined, PAGE);
      this.setPage(chatId, section, { messages, more: messages.length === PAGE, loading: false });
    } catch {
      this.setPage(chatId, section, { messages: had?.messages ?? [], more: false, loading: false });
    }
  }

  /** The page older than what is shown. */
  async more(chatId: string, section: SharedSection) {
    const had = this.page(chatId, section);
    if (!had || had.loading || !had.more) return;
    const oldest = had.messages[had.messages.length - 1];
    this.setPage(chatId, section, { ...had, loading: true });
    try {
      const older = await messengerApi.chats.shared(chatId, section, oldest?.created_at, PAGE);
      const seen = new Set(had.messages.map((m) => m.id));
      this.setPage(chatId, section, {
        messages: [...had.messages, ...older.filter((m) => !seen.has(m.id))],
        more: older.length === PAGE,
        loading: false,
      });
    } catch {
      this.setPage(chatId, section, { ...had, loading: false });
    }
  }

  /** Something in the chat changed: counts and read sections are read again. */
  changed(chatId: string) {
    if (!this.watched.has(chatId)) {
      if (this.chats[chatId]) {
        const { [chatId]: _gone, ...rest } = this.chats;
        this.chats = rest;
      }
      return;
    }
    if (this.timers.has(chatId)) return;
    this.timers.set(chatId, setTimeout(() => {
      this.timers.delete(chatId);
      this.loadCounts(chatId).catch(() => {});
      const read = Object.keys(this.chats[chatId]?.sections ?? {}) as SharedSection[];
      for (const s of read) this.open(chatId, s).catch(() => {});
    }, SETTLE_MS));
  }

  /** Runtime event → state. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    const p = (ev.payload ?? {}) as Record<string, unknown>;
    switch (ev.name) {
      case 'dm.message': {
        const m = p.message as MessengerMessage | undefined;
        if (m) this.changed(m.chat_id);
        break;
      }
      case 'dm.updated':
      case 'group.updated':
        if (typeof p.chat_id === 'string') this.changed(p.chat_id);
        break;
      case 'history.synced':
        for (const id of Object.keys(this.chats)) this.changed(id);
        break;
    }
  }

  reset() {
    for (const t of this.timers.values()) clearTimeout(t);
    this.timers.clear();
    this.chats = {};
  }
}

export const sharedStore = new SharedStore();

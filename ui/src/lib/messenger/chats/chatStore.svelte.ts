// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Chat list and the open conversation. Fed by runtime events that the
// module store forwards here; knows nothing about identity or relays.

import {
  messengerApi,
  type BotCard,
  type DmAction,
  type MessengerChat,
  type MessengerMessage,
  type MessengerPoster,
  type MessengerRecording,
  type MessengerUiEvent,
} from '../api';
import { usageStore } from '../shared/emoji/usageStore.svelte';

const PAGE = 50;
/** How many of the newest messages are read to find the card a chat ends with (system lines stand among them). */
const TAIL_PAGE = 8;

/** The card of the bot a chat ends with, and when it was written. */
export interface TailCard {
  at: number;
  card: BotCard;
}

/**
 * The runtime adds a contact on its own (the first message to a stranger, a
 * request or an accept) and says nothing of it, so the new chat dialog would
 * not find that person. The module store imports this one: reached late.
 */
function refreshContacts() {
  import('../store.svelte').then((m) => m.messengerStore.refreshContacts()).catch(() => {});
}

/**
 * The peer's profile is asked again at every opening of a chat of two, from
 * the list, a notification or a link alike: a name changed since the start
 * comes in while the chat is shown. In the background; the reply is a
 * `profile.updated` the names follow.
 */
function askProfile(chatId: string) {
  if (!chatId.startsWith('dm:')) return;
  const peer = chatId.slice(3);
  Promise.resolve()
    .then(() => messengerApi.profiles.request(peer))
    .catch(() => {});
}

class ChatStore {
  chats = $state<MessengerChat[]>([]);
  activeId = $state<string | null>(null);
  messages = $state<MessengerMessage[]>([]);
  hasMore = $state(false);
  loading = $state(false);
  loadingOlder = $state(false);
  /** Bumped whenever a message lands at the bottom; the view scrolls on it. */
  tailTick = $state(0);
  /** A message to bring into view once its chat shows it (the list of transfers asks). */
  jumpTo = $state<{ chatId: string; messageId: string } | null>(null);
  /** Bumped when the store is emptied (lock, logout): a page showing a chat loads it again. */
  resets = $state(0);
  /**
   * Chats whose newest message is a card of the bot of Veydan Cloud. A card
   * has no text, so the runtime gives the list no preview of it: the list
   * and a notice word it by its type from here.
   */
  tailCards = $state<Record<string, TailCard>>({});
  /** The newest message of a chat without a preview, as last looked at: asked once per message. */
  private _tailsSeen = new Map<string, number>();
  private _refreshTimer: ReturnType<typeof setTimeout> | null = null;
  private _reloadTimer: ReturnType<typeof setTimeout> | null = null;

  get active(): MessengerChat | null {
    return this.chats.find((c) => c.id === this.activeId) ?? null;
  }

  get totalUnread(): number {
    // Muted chats keep their own counter but stay out of the total.
    return this.chats.reduce((n, c) => n + (c.archived || c.is_muted ? 0 : c.unread), 0);
  }

  async loadChats() {
    this.chats = await messengerApi.chats.list(true);
    this.readTails().catch(() => {});
  }

  /**
   * The card `chat` ends with, when its newest message is one. A chat whose
   * newest message has words of its own has its preview and no card here.
   */
  tailCard(chat: Pick<MessengerChat, 'id' | 'last_preview' | 'last_message_at'>): BotCard | null {
    if (chat.last_preview != null || chat.last_message_at == null) return null;
    const tail = this.tailCards[chat.id];
    return tail && tail.at === chat.last_message_at ? tail.card : null;
  }

  /**
   * Chats of two that end with a message without a preview: their newest
   * messages are read once to see whether that message is a card. Only the
   * chat with the bot (and a message of a kind this app does not know) is
   * ever asked.
   */
  private async readTails() {
    for (const c of this.chats) {
      const at = c.last_message_at;
      if (c.kind !== 'dm' || c.last_preview != null || at == null) continue;
      if (this.tailCards[c.id]?.at === at || this._tailsSeen.get(c.id) === at) continue;
      this._tailsSeen.set(c.id, at);
      let page: MessengerMessage[];
      try {
        page = await messengerApi.chats.messages(c.id, undefined, TAIL_PAGE);
      } catch {
        this._tailsSeen.delete(c.id);
        continue;
      }
      // A message came while the chat was read: this page is of a moment before it.
      if (this._tailsSeen.get(c.id) !== at) continue;
      // What was read is what the chat ends with: a card kept of a later
      // message, removed since, gives way to it.
      const newest = page.findLast((m) => m.content_type !== 'system' && !m.deleted);
      this.setTail(c.id, newest?.bot_card ? { at: newest.created_at, card: newest.bot_card } : null);
    }
  }

  /** A message as it comes: a card of the bot is kept as the end of its chat, anything later than a kept card drops it. */
  private noteTail(m: MessengerMessage) {
    if (m.content_type === 'system') return;
    // The chat moved: what was read of its end is of another moment, and
    // is read again if the list comes back to it (the message removed).
    this._tailsSeen.delete(m.chat_id);
    const kept = this.tailCards[m.chat_id];
    if (m.bot_card && !m.deleted) {
      if (!kept || m.created_at >= kept.at) this.setTail(m.chat_id, { at: m.created_at, card: m.bot_card });
    } else if (kept && m.created_at >= kept.at) {
      this.setTail(m.chat_id, null);
    }
  }

  private setTail(chatId: string, tail: TailCard | null) {
    const { [chatId]: _was, ...rest } = this.tailCards;
    this.tailCards = tail ? { ...rest, [chatId]: tail } : rest;
  }

  /** Coalesce bursts (history sync delivers hundreds of events). */
  scheduleChatsRefresh() {
    if (this._refreshTimer) return;
    this._refreshTimer = setTimeout(() => {
      this._refreshTimer = null;
      this.loadChats().catch(() => {});
    }, 200);
  }

  private scheduleWindowReload() {
    if (this._reloadTimer) return;
    this._reloadTimer = setTimeout(() => {
      this._reloadTimer = null;
      this.reloadWindow().catch(() => {});
    }, 150);
  }

  /** `askProfile: false` when the runtime asked already (`openPeer`). */
  async open(chatId: string, opts: { askProfile?: boolean } = {}) {
    if (opts.askProfile !== false) askProfile(chatId);
    this.activeId = chatId;
    this.messages = [];
    this.hasMore = false;
    this.loading = true;
    try {
      const page = await messengerApi.chats.messages(chatId, undefined, PAGE);
      if (this.activeId !== chatId) return;
      this.messages = page;
      this.hasMore = page.length === PAGE;
      this.tailTick++;
      await this.markRead(chatId);
    } finally {
      this.loading = false;
    }
  }

  async openPeer(peer: string) {
    const chat = await messengerApi.chats.open(peer);
    await this.loadChats();
    // `messenger_chat_open` asked the profile.
    await this.open(chat.id, { askProfile: false });
    refreshContacts();
    return chat;
  }

  close() {
    this.activeId = null;
    this.messages = [];
  }

  async markRead(chatId: string) {
    const c = this.chats.find((x) => x.id === chatId);
    if (c && c.unread === 0) return;
    await messengerApi.chats.markRead(chatId);
    this.chats = this.chats.map((x) => (x.id === chatId ? { ...x, unread: 0 } : x));
  }

  async loadOlder() {
    const id = this.activeId;
    if (!id || !this.hasMore || this.loadingOlder || this.messages.length === 0) return;
    this.loadingOlder = true;
    try {
      const page = await messengerApi.chats.messages(id, this.messages[0].created_at, PAGE);
      if (this.activeId !== id) return;
      const known = new Set(this.messages.map((m) => m.id));
      this.messages = [...page.filter((m) => !known.has(m.id)), ...this.messages];
      this.hasMore = page.length === PAGE;
    } finally {
      this.loadingOlder = false;
    }
  }

  /**
   * Older pages of the open chat until `messageId` is among the messages
   * (a message the list of transfers asks for may be far up), or until the
   * chat has no more. Whether it is shown now.
   */
  async reach(messageId: string): Promise<boolean> {
    const id = this.activeId;
    while (this.activeId === id && !this.messages.some((m) => m.id === messageId)) {
      if (!this.hasMore || this.messages.length === 0) return false;
      // A page the scroll asked for is on its way: it is waited for.
      if (this.loadingOlder) await new Promise((r) => setTimeout(r, 50));
      else await this.loadOlder();
    }
    return this.activeId === id;
  }

  /** Re-read the loaded window (statuses, edits, deletes). */
  async reloadWindow() {
    const id = this.activeId;
    if (!id) return;
    const size = Math.max(PAGE, this.messages.length);
    const page = await messengerApi.chats.messages(id, undefined, size);
    if (this.activeId !== id) return;
    const grew = page.length > 0 && page[page.length - 1].id !== this.messages[this.messages.length - 1]?.id;
    this.messages = page;
    if (grew) this.tailTick++;
  }

  private upsert(m: MessengerMessage) {
    const i = this.messages.findIndex((x) => x.id === m.id);
    if (i >= 0) {
      this.messages = this.messages.map((x) => (x.id === m.id ? m : x));
      return;
    }
    const next = [...this.messages, m].sort((a, b) => a.created_at - b.created_at || a.id.localeCompare(b.id));
    const atTail = next[next.length - 1].id === m.id;
    this.messages = next;
    if (atTail) this.tailTick++;
  }

  /** Where a message for this chat goes: the peer, or the group. */
  private target(chat: MessengerChat | null): string {
    const to = chat?.peer_pubkey ?? (chat?.kind === "group" ? chat.id : null);
    if (!to) throw new Error("no chat");
    return to;
  }

  async send(text: string, replyTo?: string) {
    const chat = this.active;
    const to = this.target(chat);
    if (!chat) throw new Error("no chat");
    const m = await messengerApi.dm.sendText(to, text, replyTo);
    if (this.activeId === chat.id) this.upsert(m);
    this.scheduleChatsRefresh();
    if (chat.kind === 'dm' && !chat.is_contact) refreshContacts();
    return m;
  }

  /** A message into a chat that need not be the open one. */
  async sendTo(chat: MessengerChat, text: string) {
    const m = await messengerApi.dm.sendText(this.target(chat), text);
    if (this.activeId === chat.id) this.upsert(m);
    this.scheduleChatsRefresh();
    if (chat.kind === 'dm' && !chat.is_contact) refreshContacts();
    return m;
  }

  /**
   * A contact card into `chat`: mine when `pubkey` is `null` (with my phone
   * when `includePhone`), else that person's public profile.
   * Errors: `phone_public_group`, `card_too_large`, a relationship code.
   */
  async sendCard(chat: MessengerChat, pubkey: string | null, includePhone: boolean) {
    const m = await messengerApi.cards.send(chat.id, pubkey, includePhone);
    if (this.activeId === chat.id) this.upsert(m);
    this.scheduleChatsRefresh();
    return m;
  }

  /** "Add contact" on a received card; the message comes back with the person a contact. */
  async acceptCard(messageId: string) {
    const m = await messengerApi.cards.accept(messageId);
    if (m.chat_id === this.activeId) this.upsert(m);
    return m;
  }

  /**
   * Attach a local file; the placeholder appears at once. Files of one `batch` are shown together.
   * `original`: a picture goes as it is, not compressed.
   */
  async sendFile(path: string, caption?: string, batch?: string, original = false, poster: MessengerPoster | null = null) {
    const chat = this.active;
    const to = this.target(chat);
    if (!chat) throw new Error("no chat");
    const m = await messengerApi.media.sendFile(to, path, caption, batch, original, poster);
    if (this.activeId === chat.id) this.upsert(m);
    this.scheduleChatsRefresh();
    return m;
  }

  async sendRecording(rec: MessengerRecording) {
    const chat = this.active;
    const to = this.target(chat);
    if (!chat) throw new Error("no chat");
    const m = await messengerApi.media.sendRecording(to, rec);
    if (this.activeId === chat.id) this.upsert(m);
    this.scheduleChatsRefresh();
    return m;
  }

  async edit(messageId: string, text: string) {
    const m = await messengerApi.dm.edit(messageId, text);
    this.upsert(m);
    this.scheduleChatsRefresh();
  }

  /**
   * Puts my `emoji` on a message, or takes it back when it is already mine.
   * A refusal (`reaction_limit`, `reaction_invalid`, a relationship code)
   * is thrown for the window to explain, as a refused send is.
   */
  async react(messageId: string, emoji: string) {
    const m = await messengerApi.dm.react(messageId, emoji);
    if (m.chat_id === this.activeId) this.upsert(m);
    // A reaction put counts toward popular emoji in the runtime: read the new order.
    if (m.reactions.some((r) => r.emoji === emoji && r.mine)) usageStore.load().catch(() => {});
    return m;
  }

  async remove(messageId: string, forEveryone: boolean) {
    await messengerApi.dm.delete(messageId, forEveryone);
    await this.reloadWindow();
    this.scheduleChatsRefresh();
  }

  async retry(messageId: string) {
    await messengerApi.dm.retry(messageId);
    await this.reloadWindow();
  }

  /** Relationship action on the peer of a chat. */
  async act(chatId: string, action: DmAction) {
    const chat = this.chats.find((c) => c.id === chatId);
    if (!chat?.peer_pubkey) throw new Error('no chat');
    await messengerApi.dm.action(chat.peer_pubkey, action);
    await this.loadChats();
    if (this.activeId === chatId) await this.reloadWindow();
    refreshContacts();
  }

  async setPinned(chatId: string, pinned: boolean) {
    await messengerApi.chats.setPinned(chatId, pinned);
    await this.loadChats();
  }

  async setArchived(chatId: string, archived: boolean) {
    await messengerApi.chats.setArchived(chatId, archived);
    if (archived && this.activeId === chatId) this.close();
    await this.loadChats();
  }

  async setMuted(chatId: string, muted: boolean) {
    await messengerApi.chats.setMuted(chatId, muted);
    await this.loadChats();
  }

  async deleteChat(chatId: string) {
    await messengerApi.chats.delete(chatId);
    if (this.activeId === chatId) this.close();
    await this.loadChats();
  }

  /** Runtime event → state. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    const p = (ev.payload ?? {}) as Record<string, unknown>;
    switch (ev.name) {
      case 'dm.message': {
        const m = p.message as MessengerMessage | undefined;
        // Before the notice of the same message: a card is worded by its type there.
        if (m) this.noteTail(m);
        if (m && m.chat_id === this.activeId) {
          this.upsert(m);
          if (m.direction === 'in' && document.visibilityState === 'visible') {
            messengerApi.chats.markRead(m.chat_id).catch(() => {});
          }
        }
        this.scheduleChatsRefresh();
        break;
      }
      case 'dm.updated':
        if (p.chat_id === this.activeId) this.scheduleWindowReload();
        this.scheduleChatsRefresh();
        break;
      case 'dm.relationship':
        if (p.chat_id === this.activeId) this.scheduleWindowReload();
        this.scheduleChatsRefresh();
        break;
      case "group.updated":
        if (p.chat_id === this.activeId) this.scheduleWindowReload();
        this.scheduleChatsRefresh();
        break;
      // Someone read up to here: the ticks of the open chat change, its place in the list does not.
      case 'chat.receipt':
        if (p.chat_id === this.activeId) this.scheduleWindowReload();
        break;
      case "group.invite":
      case "group.request":
      case 'chats.updated':
      case 'chat.read':
      case 'profile.updated':
      case 'history.synced':
        this.scheduleChatsRefresh();
        break;
    }
  }

  reset() {
    this.chats = [];
    this.tailCards = {};
    this._tailsSeen.clear();
    this.close();
    this.resets++;
  }
}

export const chatStore = new ChatStore();

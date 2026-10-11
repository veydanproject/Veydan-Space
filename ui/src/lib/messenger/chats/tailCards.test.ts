// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A chat that ends with a card of the bot of Veydan Cloud has no preview
// from the runtime (a card has no text): the store finds the card, so the
// list and a notice word it by its type instead of «No messages».

import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { MessengerChat, MessengerMessage } from '../api';

const BOT = 'dm:' + 'bd'.repeat(32);

const api = vi.hoisted(() => ({
  list: vi.fn(async (): Promise<unknown[]> => []),
  messages: vi.fn(async (_chat: string, _before: number | undefined, _limit: number): Promise<unknown[]> => []),
}));
vi.mock('../api', () => ({
  messengerApi: {
    chats: { list: api.list, messages: api.messages, markRead: vi.fn(async () => {}) },
    profiles: { request: vi.fn(async () => {}) },
  },
}));
vi.mock('../store.svelte', () => ({ messengerStore: { refreshContacts: vi.fn(async () => {}) } }));

const { chatStore } = await import('./chatStore.svelte');

const chat = (over: Partial<MessengerChat>): MessengerChat =>
  ({ id: BOT, kind: 'dm', last_preview: null, last_message_at: 100, unread: 0, ...over }) as MessengerChat;
const message = (over: Partial<MessengerMessage>): MessengerMessage =>
  ({ id: 'm1', chat_id: BOT, direction: 'in', content_type: 'bot_card', text: null, created_at: 100, deleted: false, bot_card: null, ...over }) as MessengerMessage;
const restored = { t: 'recovery.restored', at: 90 } as const;

describe('the card a chat ends with', () => {
  beforeEach(() => {
    chatStore.reset();
    api.list.mockReset();
    api.messages.mockReset();
    api.messages.mockResolvedValue([]);
  });

  it('is read once for a chat of two that has a message and no preview', async () => {
    api.list.mockResolvedValue([chat({})]);
    api.messages.mockResolvedValue([
      message({ id: 'm0', content_type: 'text', text: 'eu-1: fine again', created_at: 50 }),
      message({ bot_card: restored }),
      message({ id: 's1', content_type: 'system', text: 'blocked', created_at: 101 }),
    ]);
    await chatStore.loadChats();
    await vi.waitFor(() => expect(chatStore.tailCard(chatStore.chats[0])).toEqual(restored));
    await chatStore.loadChats();
    await chatStore.loadChats();
    expect(api.messages).toHaveBeenCalledTimes(1);
  });

  it('is not asked for where the list has a preview, a group, or no message at all', async () => {
    api.list.mockResolvedValue([
      chat({ id: 'dm:aa', last_preview: 'hello' }),
      chat({ id: 'group:g', kind: 'group' }),
      chat({ id: 'dm:bb', last_message_at: null }),
    ]);
    await chatStore.loadChats();
    await Promise.resolve();
    expect(api.messages).not.toHaveBeenCalled();
    for (const c of chatStore.chats) expect(chatStore.tailCard(c)).toBeNull();
  });

  it('is none where the newest message is no card', async () => {
    api.list.mockResolvedValue([chat({ id: 'dm:cc' })]);
    api.messages.mockResolvedValue([message({ chat_id: 'dm:cc', content_type: 'sticker' })]);
    await chatStore.loadChats();
    await vi.waitFor(() => expect(api.messages).toHaveBeenCalledTimes(1));
    expect(chatStore.tailCard(chatStore.chats[0])).toBeNull();
  });

  it('comes with the message itself, before its notice', () => {
    chatStore.handleEvent({ name: 'dm.message', payload: { chat_id: BOT, message: message({ bot_card: restored }) } } as never);
    expect(chatStore.tailCards[BOT]).toEqual({ at: 100, card: restored });
    expect(chatStore.tailCard(chat({}))).toEqual(restored);
    // The list is behind the message, or ahead of it: the card of another moment is not shown.
    expect(chatStore.tailCard(chat({ last_message_at: 99 }))).toBeNull();
    expect(chatStore.tailCard(chat({ last_preview: 'eu-1: fine again', last_message_at: 100 }))).toBeNull();
  });

  it('goes when the bot writes words after it, and stays when an older message comes late', () => {
    chatStore.handleEvent({ name: 'dm.message', payload: { message: message({ bot_card: restored }) } } as never);
    chatStore.handleEvent({ name: 'dm.message', payload: { message: message({ id: 'old', content_type: 'text', text: 'x', created_at: 40 }) } } as never);
    expect(chatStore.tailCards[BOT]?.card).toEqual(restored);
    chatStore.handleEvent({ name: 'dm.message', payload: { message: message({ id: 'new', content_type: 'text', text: 'y', created_at: 120 }) } } as never);
    expect(chatStore.tailCards[BOT]).toBeUndefined();
  });

  it('is the older card again when the newest one is removed', async () => {
    const older = { t: 'recovery.escrow_read', at: 80 } as const;
    const first = message({ id: 'm0', bot_card: older, created_at: 90 });
    // Both read from the list, then the newest removed for me: the list is back at the older one.
    api.list.mockResolvedValue([chat({})]);
    api.messages.mockResolvedValue([first, message({ bot_card: restored })]);
    await chatStore.loadChats();
    await vi.waitFor(() => expect(chatStore.tailCard(chatStore.chats[0])).toEqual(restored));
    api.list.mockResolvedValue([chat({ last_message_at: 90 })]);
    api.messages.mockResolvedValue([first]);
    await chatStore.loadChats();
    await vi.waitFor(() => expect(chatStore.tailCard(chatStore.chats[0])).toEqual(older));

    // The older one read from the list, the newest come by itself and removed: read again, though the list was here before.
    chatStore.handleEvent({ name: 'dm.message', payload: { message: message({ bot_card: restored }) } } as never);
    expect(chatStore.tailCard(chat({}))).toEqual(restored);
    await chatStore.loadChats();
    await vi.waitFor(() => expect(chatStore.tailCard(chatStore.chats[0])).toEqual(older));
  });

  it('is back when the words written after it are removed', async () => {
    api.list.mockResolvedValue([chat({})]);
    api.messages.mockResolvedValue([message({ bot_card: restored })]);
    await chatStore.loadChats();
    await vi.waitFor(() => expect(chatStore.tailCard(chatStore.chats[0])).toEqual(restored));
    chatStore.handleEvent({ name: 'dm.message', payload: { message: message({ id: 'new', content_type: 'text', text: 'y', created_at: 120 }) } } as never);
    expect(chatStore.tailCards[BOT]).toBeUndefined();
    // The text is taken back: the list ends with the card again, at the time it was read at before.
    await chatStore.loadChats();
    await vi.waitFor(() => expect(chatStore.tailCard(chatStore.chats[0])).toEqual(restored));
  });

  it('is none once the chat ends with something else, whatever was kept', async () => {
    chatStore.handleEvent({ name: 'dm.message', payload: { message: message({ bot_card: restored }) } } as never);
    // The card removed; the chat now ends with an older message of a kind this app does not know.
    api.list.mockResolvedValue([chat({ last_message_at: 90 })]);
    api.messages.mockResolvedValue([message({ id: 'm0', content_type: 'sticker', created_at: 90 })]);
    await chatStore.loadChats();
    await vi.waitFor(() => expect(chatStore.tailCards[BOT]).toBeUndefined());
  });

  it('is forgotten with the store', () => {
    chatStore.handleEvent({ name: 'dm.message', payload: { message: message({ bot_card: restored }) } } as never);
    chatStore.reset();
    expect(chatStore.tailCards).toEqual({});
  });
});

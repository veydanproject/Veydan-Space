// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A message the list of transfers asks for may be far up its chat: older
// pages are read until it is shown, or until the chat has no more.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => {
  // 120 messages, the newest last; a page is the `limit` before `before`.
  const all = Array.from({ length: 120 }, (_, n) => ({ id: `m${n}`, chat_id: 'dm:x', created_at: n + 1 }));
  const messages = vi.fn(async (_chat: string, before: number | undefined, limit: number) => {
    const older = all.filter((m) => before === undefined || m.created_at < before);
    return older.slice(Math.max(0, older.length - limit));
  });
  const requestProfile = vi.fn(async (_pubkey: string) => {});
  const markRead = vi.fn(async (_chat: string) => {});
  return { messages, requestProfile, markRead };
});
vi.mock('../api', () => ({
  messengerApi: {
    chats: { messages: api.messages, markRead: api.markRead, list: vi.fn(async () => []), open: vi.fn(async () => ({ id: 'dm:x' })), delete: vi.fn(async () => {}) },
    profiles: { request: api.requestProfile },
  },
}));

const store = vi.hoisted(() => ({ refreshContacts: vi.fn(async () => {}) }));
vi.mock('../store.svelte', () => ({ messengerStore: store }));

const { chatStore, inSight, lockSight } = await import('./chatStore.svelte');

describe('reaching a message of the open chat', () => {
  beforeEach(async () => {
    api.messages.mockClear();
    chatStore.chats = [{ id: 'dm:x', unread: 0 } as never];
    await chatStore.open('dm:x');
  });

  it('finds one among the last page without reading more', async () => {
    expect(await chatStore.reach('m110')).toBe(true);
    expect(api.messages).toHaveBeenCalledTimes(1);
  });

  it('reads older pages until the message is shown', async () => {
    expect(chatStore.messages.some((m) => m.id === 'm5')).toBe(false);
    expect(await chatStore.reach('m5')).toBe(true);
    expect(chatStore.messages.some((m) => m.id === 'm5')).toBe(true);
    expect(chatStore.hasMore).toBe(false);
  });

  it('stops when the chat has no more', async () => {
    expect(await chatStore.reach('gone')).toBe(false);
    expect(chatStore.messages).toHaveLength(120);
  });
});

describe('contacts the runtime adds on its own', () => {
  it('are read again once a chat with a peer is opened', async () => {
    store.refreshContacts.mockClear();
    await chatStore.openPeer('ab'.repeat(32));
    await vi.waitFor(() => expect(store.refreshContacts).toHaveBeenCalledTimes(1));
  });
});

describe('the profile of the peer', () => {
  beforeEach(() => api.requestProfile.mockClear());

  it('is asked again when a chat of two is opened from the list', async () => {
    const peer = 'cd'.repeat(32);
    chatStore.chats = [{ id: `dm:${peer}`, unread: 0 } as never];
    await chatStore.open(`dm:${peer}`);
    await vi.waitFor(() => expect(api.requestProfile).toHaveBeenCalledWith(peer));
    expect(api.requestProfile).toHaveBeenCalledTimes(1);
  });

  it('is not asked for a group, nor twice when the runtime opened the chat', async () => {
    await chatStore.open('group:g1');
    await chatStore.openPeer('ab'.repeat(32));
    await new Promise((r) => setTimeout(r, 0));
    expect(api.requestProfile).not.toHaveBeenCalled();
  });

  it('failing to be asked does not stop the opening', async () => {
    api.requestProfile.mockRejectedValueOnce(new Error('no session'));
    chatStore.chats = [{ id: 'dm:x', unread: 0 } as never];
    await chatStore.open('dm:x');
    expect(chatStore.messages).toHaveLength(50);
  });
});

describe('the resets of the store', () => {
  it('go up when the store is emptied, not when a chat is closed', async () => {
    chatStore.chats = [{ id: 'dm:x', unread: 0 } as never];
    await chatStore.open('dm:x');
    const before = chatStore.resets;
    chatStore.close();
    expect(chatStore.resets).toBe(before);
    chatStore.reset();
    expect(chatStore.resets).toBe(before + 1);
    expect(chatStore.chats).toHaveLength(0);
    expect(chatStore.activeId).toBeNull();
  });

  it('do not move on a delete of the open chat', async () => {
    chatStore.chats = [{ id: 'dm:x', unread: 0 } as never];
    await chatStore.open('dm:x');
    const before = chatStore.resets;
    await chatStore.deleteChat('dm:x');
    expect(chatStore.activeId).toBeNull();
    expect(chatStore.resets).toBe(before);
  });
});

describe('behind the lock', () => {
  beforeEach(() => {
    api.markRead.mockClear();
    lockSight.locked = false;
    // The screen of the lock leaves the document visible.
    vi.stubGlobal('document', { visibilityState: 'visible' });
  });
  afterEach(() => {
    lockSight.locked = false;
    vi.unstubAllGlobals();
  });

  it('marks nothing read: neither the chat that opens nor what comes in the open one', async () => {
    chatStore.chats = [{ id: 'dm:x', unread: 2 } as never];
    await chatStore.open('dm:x');
    expect(api.markRead).toHaveBeenCalledTimes(1);
    api.markRead.mockClear();
    chatStore.chats = [{ id: 'dm:x', unread: 2 } as never];

    lockSight.locked = true;
    await chatStore.markRead('dm:x');
    const m = { id: 'in1', chat_id: 'dm:x', direction: 'in', created_at: 500, content_type: 'text' };
    chatStore.handleEvent({ name: 'dm.message', payload: { message: m } } as never);
    expect(api.markRead).not.toHaveBeenCalled();
    expect(chatStore.chats[0].unread).toBe(2);

    lockSight.locked = false;
    await chatStore.markRead('dm:x');
    expect(api.markRead).toHaveBeenCalledWith('dm:x');
  });

  it('is not the page in sight, though the document is visible', () => {
    expect(inSight()).toBe(true);
    lockSight.locked = true;
    expect(inSight()).toBe(false);
    lockSight.locked = false;
    vi.stubGlobal('document', { visibilityState: 'hidden' });
    expect(inSight()).toBe(false);
  });
});

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A message the list of transfers asks for may be far up its chat: older
// pages are read until it is shown, or until the chat has no more.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => {
  // 120 messages, the newest last; a page is the `limit` before `before`.
  const all = Array.from({ length: 120 }, (_, n) => ({ id: `m${n}`, chat_id: 'dm:x', created_at: n + 1 }));
  const messages = vi.fn(async (_chat: string, before: number | undefined, limit: number) => {
    const older = all.filter((m) => before === undefined || m.created_at < before);
    return older.slice(Math.max(0, older.length - limit));
  });
  return { messages };
});
vi.mock('../api', () => ({
  messengerApi: { chats: { messages: api.messages, markRead: vi.fn(async () => {}), list: vi.fn(async () => []) } },
}));

const { chatStore } = await import('./chatStore.svelte');

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

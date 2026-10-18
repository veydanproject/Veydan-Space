// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The requests of strangers as the store keeps them: read from the runtime,
// answered one by one, read again on every event that may have moved them.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const req = (n: number, extra: Record<string, unknown> = {}) => ({
  chat_id: `dm:${String(n).repeat(64).slice(0, 64)}`, peer: String(n).repeat(64).slice(0, 64), name: `p${n}`, text: `hello ${n}`, at: 1000 - n, held: 0, ...extra,
});

const api = vi.hoisted(() => ({
  requests: vi.fn(),
  requestAct: vi.fn(async (_peer: string, _action: string) => {}),
}));
vi.mock('../api', () => ({ messengerApi: { moderation: { requests: api.requests, requestAct: api.requestAct } } }));

const chats = vi.hoisted(() => ({ loadChats: vi.fn(async () => {}), close: vi.fn() }));
vi.mock('../chats/chatStore.svelte', () => ({ chatStore: chats }));

const store = vi.hoisted(() => ({ refreshContacts: vi.fn(async () => {}) }));
vi.mock('../store.svelte', () => ({ messengerStore: store }));

const { requestStore, requestsUi, requestName, firstLine } = await import('./requestStore.svelte');

beforeEach(() => {
  requestStore.reset();
  api.requests.mockReset();
  api.requestAct.mockReset();
  chats.loadChats.mockClear();
  chats.close.mockClear();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('reading the requests', () => {
  it('takes the first page, the total and where the next page starts', async () => {
    api.requests.mockResolvedValueOnce({ items: [req(1), req(2)], next_before: 998, total: 3 });
    await requestStore.load();
    expect(api.requests).toHaveBeenCalledWith(undefined, 50);
    expect(requestStore.items.map((r) => r.name)).toEqual(['p1', 'p2']);
    expect(requestStore.count).toBe(3);
    expect(requestStore.nextBefore).toBe(998);
    expect(requestStore.loaded).toBe(true);
  });

  it('adds the next page without repeating a request that moved between pages', async () => {
    api.requests.mockResolvedValueOnce({ items: [req(1), req(2)], next_before: 998, total: 3 });
    await requestStore.load();
    api.requests.mockResolvedValueOnce({ items: [req(2), req(3)], total: 3 });
    await requestStore.loadMore();
    expect(api.requests).toHaveBeenLastCalledWith(998, 50);
    expect(requestStore.items.map((r) => r.name)).toEqual(['p1', 'p2', 'p3']);
    expect(requestStore.nextBefore).toBeNull();
  });

  it('reads no further at the end of the queue, nor twice at once', async () => {
    api.requests.mockResolvedValueOnce({ items: [req(1)], total: 1 });
    await requestStore.load();
    expect(requestStore.nextBefore).toBeNull();
    await requestStore.loadMore();
    expect(api.requests).toHaveBeenCalledTimes(1);

    requestStore.nextBefore = 998;
    let answer!: (v: unknown) => void;
    api.requests.mockReturnValueOnce(new Promise((r) => (answer = r)));
    const first = requestStore.loadMore();
    expect(requestStore.loadingMore).toBe(true);
    await requestStore.loadMore();
    expect(api.requests).toHaveBeenCalledTimes(2);
    answer({ items: [req(2)], total: 2 });
    await first;
    expect(requestStore.loadingMore).toBe(false);
    expect(requestStore.items.map((r) => r.name)).toEqual(['p1', 'p2']);
  });

  it('drops the next page of a read started before the store was emptied', async () => {
    api.requests.mockResolvedValueOnce({ items: [req(1)], next_before: 999, total: 2 });
    await requestStore.load();
    let answer!: (v: unknown) => void;
    api.requests.mockReturnValueOnce(new Promise((r) => (answer = r)));
    const reading = requestStore.loadMore();
    requestStore.reset();
    answer({ items: [req(2)], total: 2 });
    await reading;
    expect(requestStore.items).toEqual([]);
    expect(requestStore.loadingMore).toBe(false);
  });

  it('reads again as many as are shown, so a reload does not shorten the list', async () => {
    requestStore.items = Array.from({ length: 70 }, (_, n) => req(n + 1)) as never;
    api.requests.mockResolvedValueOnce({ items: [], total: 0 });
    await requestStore.load();
    expect(api.requests).toHaveBeenCalledWith(undefined, 70);
  });

  it('drops an answer to a read started before the store was emptied', async () => {
    let answer!: (v: unknown) => void;
    api.requests.mockReturnValueOnce(new Promise((r) => (answer = r)));
    const reading = requestStore.load();
    requestStore.reset();
    answer({ items: [req(1)], total: 1 });
    await reading;
    expect(requestStore.items).toEqual([]);
    expect(requestStore.count).toBe(0);
  });
});

describe('answering a request', () => {
  beforeEach(async () => {
    api.requests.mockResolvedValue({ items: [req(1), req(2)], total: 2 });
    await requestStore.load();
    requestStore.selected = req(1).peer;
  });

  it('takes it out of the list at once and reads the chats again', async () => {
    await requestStore.act(req(1).peer, 'accept');
    expect(api.requestAct).toHaveBeenCalledWith(req(1).peer, 'accept');
    expect(requestStore.items.map((r) => r.name)).toEqual(['p2']);
    expect(requestStore.count).toBe(1);
    expect(requestStore.selected).toBeNull();
    expect(requestStore.acting).toBeNull();
    expect(chats.loadChats).toHaveBeenCalled();
  });

  it('answers a request of a private chat there, not at home', async () => {
    const s1 = '0123456789abcdef';
    const priv = req(1, { chat_id: `dm:${req(1).peer}@${s1}` });
    api.requests.mockResolvedValue({ items: [priv, req(1)], total: 2 });
    await requestStore.load();
    await requestStore.act(priv.peer, 'block', priv.chat_id);
    expect(api.requestAct).toHaveBeenCalledWith(`${priv.peer}@${s1}`, 'block');
    expect(requestStore.items.map((r) => r.chat_id), 'the home request of the same person stays').toEqual([req(1).chat_id]);
  });

  it('keeps it when the runtime refuses, and the refusal reaches the view', async () => {
    api.requestAct.mockRejectedValueOnce(new Error('dm_blocked'));
    await expect(requestStore.act(req(1).peer, 'block')).rejects.toThrow('dm_blocked');
    expect(requestStore.items).toHaveLength(2);
    expect(requestStore.count).toBe(2);
    expect(requestStore.acting).toBeNull();
  });
});

describe('events', () => {
  it('show the count of `dm.request` at once and read the list once for a burst', async () => {
    vi.useFakeTimers();
    api.requests.mockResolvedValue({ items: [req(1)], total: 1 });
    requestStore.handleEvent({ name: 'dm.request', payload: { peer: req(1).peer, pending: 4 } });
    expect(requestStore.count).toBe(4);
    requestStore.handleEvent({ name: 'chats.updated', payload: {} });
    requestStore.handleEvent({ name: 'history.synced', payload: {} });
    await vi.runAllTimersAsync();
    expect(api.requests).toHaveBeenCalledTimes(1);
    expect(requestStore.count).toBe(1);
  });

  it('read the list again when a relationship moved (another device answered)', async () => {
    vi.useFakeTimers();
    api.requests.mockResolvedValue({ items: [], total: 0 });
    requestStore.handleEvent({ name: 'dm.relationship', payload: { peer: 'ab', mode: 'full_chat' } });
    await vi.runAllTimersAsync();
    expect(api.requests).toHaveBeenCalledTimes(1);
  });

  it('take a count only when it is a number', async () => {
    vi.useFakeTimers();
    api.requests.mockResolvedValue({ items: [], total: 0 });
    requestStore.count = 2;
    requestStore.handleEvent({ name: 'dm.request', payload: { peer: 'ab', pending: '7' } });
    expect(requestStore.count).toBe(2);
    requestStore.handleEvent({ name: 'dm.request', payload: null });
    expect(requestStore.count).toBe(2);
    await vi.runAllTimersAsync();
    expect(requestStore.count).toBe(0);
  });

  it('leave the store alone for events that move no request', async () => {
    vi.useFakeTimers();
    requestStore.handleEvent({ name: 'chat.receipt', payload: {} });
    await vi.runAllTimersAsync();
    expect(api.requests).not.toHaveBeenCalled();
  });
});

describe('the requests in the rail of the desktop', () => {
  it('close the open chat when they open, and forget the shown request when they close', () => {
    api.requests.mockResolvedValue({ items: [], total: 0 });
    requestsUi.open();
    expect(chats.close).toHaveBeenCalled();
    expect(requestsUi.shown).toBe(true);
    requestStore.selected = 'ab';
    requestsUi.close();
    expect(requestsUi.shown).toBe(false);
    expect(requestStore.selected).toBeNull();
  });
});

describe('words of a request', () => {
  it('name it by the runtime, else by a short form of the key', () => {
    expect(requestName({ name: ' Вера ', peer: 'ab'.repeat(32) })).toBe('Вера');
    expect(requestName({ name: '', peer: 'ab'.repeat(32) })).toBe('ababab…abab');
  });

  it('show its first line in the list', () => {
    expect(firstLine('  Hello!\nSecond line')).toBe('Hello!');
    expect(firstLine('one')).toBe('one');
  });
});

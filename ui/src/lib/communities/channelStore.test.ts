// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/groupStore.test.ts at 8ce09c39, then changed for channels.

// The events of the module name the channel and only signal: the store
// reads the channel again, once for a burst; a message bumps the revision
// a page follows; the progress of a file is kept until it is stored.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
  get: vi.fn(async (id: string) => ({ channel: { id, chat_id: `channel:${id}`, name: id }, staff: [], pinned: [] })),
  list: vi.fn(async (): Promise<unknown[]> => []),
  uploads: vi.fn(async () => []),
}));
vi.mock('./api', () => {
  const errorText = (e: unknown) => (e != null && typeof e === 'object' && 'message' in e ? String((e as { message: unknown }).message) : String(e));
  return {
    channelsApi: { get: api.get, list: api.list, uploads: api.uploads, invites: vi.fn(async () => []) },
    listenChannels: vi.fn(async () => () => {}),
    errorText,
    errorCode: (e: unknown) => (e != null && typeof e === 'object' && 'code' in e ? String((e as { code: unknown }).code) : null),
    channelErrorCode: (e: unknown) => /^(channel_[a-z_]+):/.exec(errorText(e))?.[1] ?? null,
  };
});

const { channelStore, communityOf, chatIdOf } = await import('./channelStore.svelte');
const { feedStore, feedKey } = await import('./feedStore.svelte');
const refusal = (code: string) => ({ code: 'other', message: `${code}: x` });

describe('the events of the channels', () => {
  beforeEach(() => {
    api.get.mockClear();
    channelStore.reset();
  });

  it('read the channel again, one read for a burst', async () => {
    channelStore.handleEvent({ name: 'channel.queue', payload: { channel: 'c', pending: 2 } });
    channelStore.handleEvent({ name: 'channel.moderation', payload: { channel: 'c', action: 'approve', by: 'x', target: 'm' } });
    channelStore.handleEvent({ name: 'channel.updated', payload: { community_id: 'c' } });
    expect(api.get).not.toHaveBeenCalled();
    expect(channelStore.queue.c).toBe(2);
    await vi.waitFor(() => expect(api.get).toHaveBeenCalledTimes(1));
    expect(api.get).toHaveBeenCalledWith('c');
    expect(channelStore.detail('c')?.channel.id).toBe('c');
  });

  it('are ignored without a channel named', async () => {
    channelStore.handleEvent({ name: 'channel.updated', payload: {} });
    await new Promise((r) => setTimeout(r, 300));
    expect(api.get).not.toHaveBeenCalled();
  });

  it('bump the revision of a channel at each message', () => {
    channelStore.handleEvent({ name: 'channel.message', payload: { community_id: 'c', id: 'p1' } });
    channelStore.handleEvent({ name: 'channel.message', payload: { community_id: 'c', id: 'p2' } });
    expect(channelStore.revision.c).toBe(2);
  });

  it('keep the progress of a file on its way, and drop a failed one only when cancelled', () => {
    channelStore.handleEvent({ name: 'channel.media', payload: { community_id: 'c', id: 'u1', status: 'uploading', done: 10, total: 100 } });
    expect(channelStore.uploads.u1).toMatchObject({ communityId: 'c', done: 10, total: 100 });
    channelStore.handleEvent({ name: 'channel.media', payload: { community_id: 'c', id: 'u1', status: 'failed', error: 'no host' } });
    expect(channelStore.uploads.u1).toMatchObject({ status: 'failed', error: 'no host', done: 10 });
  });

  it('do not take a download for a file of mine', () => {
    channelStore.handleEvent({ name: 'channel.media', payload: { community_id: 'c', id: 'm1', status: 'downloading', done: 1, total: 2 } });
    // Its end, failed or done, is no row either, and reads no page again.
    for (const status of ['failed', 'done']) {
      channelStore.handleEvent({ name: 'channel.media', payload: { community_id: 'c', id: 'm1', status, done: 1, total: 2, progress: { direction: 'down' } } });
    }
    expect(channelStore.uploads.m1).toBeUndefined();
    expect(channelStore.revision.c).toBeUndefined();
  });

  it('keep a paused file of mine', () => {
    channelStore.handleEvent({ name: 'channel.media', payload: { community_id: 'c', id: 'u2', status: 'uploading', done: 10, total: 100 } });
    channelStore.handleEvent({ name: 'channel.media', payload: { community_id: 'c', id: 'u2', status: 'paused' } });
    expect(channelStore.uploads.u2).toMatchObject({ status: 'paused', done: 10, total: 100 });
  });
});

describe('the page of a channel read again', () => {
  beforeEach(() => {
    api.get.mockClear();
    channelStore.reset();
  });

  it('keeps what is shown when the read fails for any reason but an unknown channel', async () => {
    await channelStore.refresh('c');
    feedStore.merge(feedKey('c'), { items: [], asked: false });
    api.get.mockRejectedValueOnce(refusal('channel_no_session'));
    await expect(channelStore.refresh('c')).rejects.toBeTruthy();
    expect(channelStore.detail('c')).not.toBeNull();
    api.get.mockRejectedValueOnce({ code: 'vault_locked', message: 'locked' });
    await expect(channelStore.refresh('c')).rejects.toBeTruthy();
    expect(channelStore.detail('c')).not.toBeNull();
    expect(feedStore.list(feedKey('c'))).not.toBeNull();
  });

  it('drops a channel this device does not know, and its feed', async () => {
    await channelStore.refresh('c');
    feedStore.merge(feedKey('c'), { items: [], asked: false });
    api.get.mockRejectedValueOnce(refusal('channel_unknown'));
    await channelStore.refresh('c');
    expect(channelStore.detail('c')).toBeNull();
    expect(channelStore.list.some((x) => x.id === 'c')).toBe(false);
    expect(feedStore.list(feedKey('c'))).toBeNull();
  });

  it('keeps the same page when it is read again unchanged', async () => {
    await channelStore.refresh('c');
    const page = channelStore.detail('c');
    const list = channelStore.list;
    await channelStore.refresh('c');
    expect(channelStore.detail('c')).toBe(page);
    expect(channelStore.list).toBe(list);
    api.get.mockResolvedValueOnce({ channel: { id: 'c', chat_id: 'channel:c', name: 'renamed' }, staff: [], pinned: [] });
    await channelStore.refresh('c');
    expect(channelStore.detail('c')).not.toBe(page);
    expect(channelStore.detail('c')?.channel.name).toBe('renamed');
  });

  it('tells the journal and the keeper apart from the messages', () => {
    channelStore.handleEvent({ name: 'channel.message', payload: { community_id: 'c', id: 'p1' } });
    expect(channelStore.journalRev.c).toBeUndefined();
    expect(channelStore.keeperRev.c).toBeUndefined();
    channelStore.handleEvent({ name: 'channel.updated', payload: { community_id: 'c' } });
    channelStore.handleEvent({ name: 'channel.moderation', payload: { channel: 'c', action: 'hide' } });
    expect(channelStore.journalRev.c).toBe(2);
    channelStore.handleEvent({ name: 'channel.keeper', payload: { community_id: 'c' } });
    expect(channelStore.keeperRev.c).toBe(1);
    expect(channelStore.journalRev.c).toBe(2);
  });

  it('does not read the page again for a message', async () => {
    channelStore.handleEvent({ name: 'channel.message', payload: { community_id: 'c', id: 'p1' } });
    await new Promise((r) => setTimeout(r, 300));
    expect(api.get).not.toHaveBeenCalled();
  });
});

describe('the rows of the channels', () => {
  beforeEach(() => {
    api.list.mockReset();
    channelStore.reset();
  });

  it('are not said to be none while the module is not open yet', async () => {
    api.list.mockRejectedValueOnce(refusal('channel_no_session'));
    await channelStore.load();
    expect(channelStore.loaded).toBe(false);
    api.list.mockResolvedValueOnce([{ id: 'c', chat_id: 'channel:c', name: 'c' }]);
    channelStore.handleEvent({ name: 'channel.ready', payload: {} });
    await vi.waitFor(() => expect(channelStore.loaded).toBe(true));
    expect(channelStore.list.map((c) => c.id)).toEqual(['c']);
  });

  it('are said to be none when the module answered so', async () => {
    api.list.mockResolvedValueOnce([]);
    await channelStore.load();
    expect(channelStore.loaded).toBe(true);
    expect(channelStore.list).toEqual([]);
  });

  it('keep what was read when a read fails otherwise', async () => {
    api.list.mockResolvedValueOnce([{ id: 'c', chat_id: 'channel:c', name: 'c' }]);
    await channelStore.load();
    api.list.mockRejectedValueOnce(new Error('busy'));
    await channelStore.load();
    expect(channelStore.loaded).toBe(true);
    expect(channelStore.list.map((c) => c.id)).toEqual(['c']);
  });
});

describe('the chat id of a channel', () => {
  it('is channel:<id> and back', () => {
    expect(chatIdOf('abc')).toBe('channel:abc');
    expect(communityOf('channel:abc')).toBe('abc');
    expect(communityOf('group:abc')).toBeNull();
    expect(communityOf('channel:')).toBeNull();
  });
});

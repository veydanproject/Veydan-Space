// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A reader asks for a level (communities-wire §14.4): which levels are
// offered, what each adds, and the request kept as sent until the
// channel changes; a moderator's count follows `channel.level_request`.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { EVERYONE } from './sections';
import { adds, clearAsked, levelFor, markAsked, requestable, waitingLevels } from './request';
import type { LevelView, ScopeView } from './types';

const api = vi.hoisted(() => ({
  request: vi.fn(async () => undefined),
  requests: vi.fn(async () => [{ requester: 'v', level: 1, note: '', created_at: 1 }, { requester: 'n', level: 2, note: '', created_at: 2 }]),
  requestAct: vi.fn(async () => undefined),
  get: vi.fn(async (id: string) => ({ channel: { id, chat_id: `channel:${id}`, name: id }, staff: [], pinned: [] })),
}));
vi.mock('../api', () => ({
  channelsApi: {
    get: api.get, list: vi.fn(async () => []), uploads: vi.fn(async () => []), invites: vi.fn(async () => []),
    moderation: { queue: vi.fn(async () => ({ items: [], total: 0 })), requestsCount: vi.fn(async () => 0) },
    levels: { request: api.request, requests: api.requests, requestAct: api.requestAct },
  },
  listenChannels: vi.fn(async () => () => {}),
}));

const { channelStore } = await import('../channelStore.svelte');

const scope = (scope_id: string, title: string): ScopeView => ({ scope_id, title, scheme: 'sd-v1', history: 'open', mine: false });
const level = (id: number, scopes: string[], can: Partial<LevelView['can']> = {}): LevelView =>
  ({ id, name: `L${id}`, scopes, can: { comment: false, discuss: false, post: false, ...can }, members: 0 });
const scopes = [scope(EVERYONE, ''), scope('premium', 'Премиум'), scope('club', 'Клуб')];
const levels = [level(0, [EVERYONE], { comment: true }), level(1, ['premium'], { comment: true }), level(2, ['premium', 'club'], { comment: true, discuss: true }), level(3, [], { comment: true })];

describe('the levels a reader may ask for', () => {
  it('are those that give more than mine: a section or a right I lack', () => {
    expect(requestable(levels, [], scopes).map((l) => l.id)).toEqual([1, 2]);
    expect(requestable(levels, [1], scopes).map((l) => l.id)).toEqual([2]);
    expect(requestable(levels, [2], scopes)).toEqual([]);
  });

  it('say what they would add to mine', () => {
    const a = adds(levels[2], levels, [1], scopes);
    expect(a.scopes.map((s) => s.title)).toEqual(['Клуб']);
    expect(a.rights).toEqual(['discuss']);
  });

  it('offer, for a locked section, the first that opens it', () => {
    expect(levelFor('club', levels, [1], scopes)?.id).toBe(2);
    expect(levelFor('premium', levels, [], scopes)?.id).toBe(1);
    expect(levelFor('staff', levels, [], scopes)).toBeNull();
  });
});

describe('a request for a level', () => {
  beforeEach(() => {
    channelStore.reset();
    api.request.mockClear();
  });

  it('waits as the channel keeps it after a restart, and as asked here', () => {
    const mine = [{ level: 2, status: 'pending' }, { level: 3, status: 'refused' }];
    expect(waitingLevels({}, 'c', mine)).toEqual([2]);
    expect(waitingLevels({ c: [3, 2] }, 'c', mine)).toEqual([3, 2]);
    expect(waitingLevels({ d: [1] }, 'c', undefined)).toEqual([]);
  });

  it('is kept by channel, once per level, and cleared for that channel alone', () => {
    let asked = markAsked({}, 'c', 1);
    asked = markAsked(asked, 'c', 1);
    asked = markAsked(asked, 'd', 2);
    expect(asked).toEqual({ c: [1], d: [2] });
    expect(clearAsked(asked, 'c')).toEqual({ d: [2] });
    expect(clearAsked(asked, 'x')).toBe(asked);
  });

  it('is sent with its note and said as sent until the channel tells of a change', async () => {
    await channelStore.askLevel('c', 1, 'Хочу читать раньше');
    expect(api.request).toHaveBeenCalledWith('c', 1, 'Хочу читать раньше');
    expect(channelStore.levelAsked.c).toEqual([1]);
    channelStore.handleEvent({ name: 'channel.message', payload: { community_id: 'c', id: 'p' } });
    expect(channelStore.levelAsked.c).toEqual([1]);
    channelStore.handleEvent({ name: 'channel.updated', payload: { community_id: 'c' } });
    expect(channelStore.levelAsked.c).toBeUndefined();
  });

  it('that failed is not said as sent', async () => {
    api.request.mockRejectedValueOnce({ code: 'other', message: 'channel_not_permitted: no' });
    await expect(channelStore.askLevel('c', 2)).rejects.toBeTruthy();
    expect(channelStore.levelAsked.c).toBeUndefined();
  });

  it('bumps the moderators’ count when the channel tells of one, and an answer takes it away', async () => {
    channelStore.handleEvent({ name: 'channel.level_request', payload: { community_id: 'c', requester: 'v' } });
    await vi.waitFor(() => expect(channelStore.levelRequests.c).toBe(2));
    api.requests.mockResolvedValueOnce([{ requester: 'n', level: 2, note: '', created_at: 2 }]);
    await channelStore.answerLevelRequest('c', 'v', 1, false, 'Пока нет');
    expect(api.requestAct).toHaveBeenCalledWith('c', 'v', 1, false, 'Пока нет');
    await vi.waitFor(() => expect(channelStore.levelRequests.c).toBe(1));
  });
});

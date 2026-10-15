// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The events of the moderation name the group, not the chat, and only
// signal: the store reads the group again, once for a burst.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
  get: vi.fn(async (id: string) => ({ id, chat_id: `group:${id}`, members: [], requests: [], queue: 2 })),
}));
vi.mock('../api', () => ({ messengerApi: { groups: { get: api.get } } }));

const { groupStore } = await import('./groupStore.svelte');

describe('a decision of a moderator, the queue grown or shrunk', () => {
  beforeEach(() => api.get.mockClear());

  it('reads the group again, one read for a burst', async () => {
    groupStore.handleEvent({ name: 'group.queue', payload: { group: 'g', pending: 2 } } as never);
    groupStore.handleEvent({ name: 'group.moderation', payload: { group: 'g', action: 'approve', by: 'x', target: 'm' } } as never);
    expect(api.get).not.toHaveBeenCalled();
    await vi.waitFor(() => expect(api.get).toHaveBeenCalledTimes(1));
    expect(api.get).toHaveBeenCalledWith('g');
    expect(groupStore.groups.g?.queue).toBe(2);
  });

  it('is ignored without a group named', async () => {
    groupStore.handleEvent({ name: 'group.queue', payload: { pending: 2 } } as never);
    await new Promise((r) => setTimeout(r, 300));
    expect(api.get).not.toHaveBeenCalled();
  });
});

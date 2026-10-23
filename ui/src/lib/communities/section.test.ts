// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The rows of «Каналы» in the list of chats go by the last post of each
// channel (owner 2026-10-23: «не по тому, когда тыкнул на него, а по
// последнему посту»), in the browser's demo as in the core: opening a
// channel, reading it, a comment, a reaction or an edit leave the order;
// a new post moves its channel up, and its time is the row's.

import { describe, expect, it } from 'vitest';
import { channelsApi, type ChannelView } from './api';
import { rowOf } from './section';

const tr = (key: string) => key;

/** The order the list of chats gives the rows (`ChatList`: pinned first, then by `at`). */
async function order(): Promise<string[]> {
  const rows = (await channelsApi.list()).map((c) => rowOf(c, tr));
  return rows.sort((a, b) => Number(b.pinned) - Number(a.pinned) || (b.at ?? 0) - (a.at ?? 0)).map((r) => r.chatId);
}

const joinedWithPosts = (list: ChannelView[]) => list.filter((c) => c.membership === 'joined' && c.last && !c.pinned);

describe('the order of the channels', () => {
  it('is that of the last post, or of the coming in with none', async () => {
    const list = await channelsApi.list();
    for (const c of list) {
      expect(c.last_at).toBe(c.last?.at ?? c.created_at);
      expect(rowOf(c, tr).at).toBe(c.last_at);
    }
    const ats = list.map((c) => c.last_at);
    expect(ats).toEqual([...ats].sort((a, b) => b - a));
  });

  it('stays when a channel is opened, read, commented, reacted to or edited', async () => {
    const before = await order();
    const below = joinedWithPosts(await channelsApi.list()).at(-1)!;
    const post = below.last!.id;
    await channelsApi.get(below.id);
    await channelsApi.history(below.id);
    await channelsApi.read(below.id);
    await channelsApi.comment(below.id, post, 'a comment');
    await channelsApi.react(below.id, post, '👍', true);
    await channelsApi.edit(below.id, post, 'edited');
    expect(await order()).toEqual(before);
  });

  it('moves a channel up by a new post, whose time and words the row shows', async () => {
    const below = joinedWithPosts(await channelsApi.list()).at(-1)!;
    await channelsApi.post(below.id, 'news of the day');
    const list = await channelsApi.list();
    const c = list.find((x) => x.id === below.id)!;
    const row = rowOf(c, tr);
    expect(row.at).toBe(c.last!.at);
    expect(row.preview).toBe('news of the day');
    const first = (await order()).find((id) => !list.find((x) => x.chat_id === id)?.pinned);
    expect(first).toBe(c.chat_id);
  });
});

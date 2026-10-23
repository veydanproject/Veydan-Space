// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The feeds as read on this device: a channel opened again shows what was
// read at once; a page read again is merged by id and leaves alone what did
// not change; a burst of reads is one more read; a list left keeps its newest.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { FeedItemView, FeedPageView } from './api';
import { KEEP_ITEMS, feedKey, feedStore, scopeOfKey, threadKey } from './feedStore.svelte';

const post = (n: number, extra: Partial<FeedItemView> = {}): FeedItemView => ({
  id: `p${String(n).padStart(3, '0')}`, community_id: 'c', post_id: '', author: 'a', created_at: 1000 + n,
  text: `post ${n}`, review: 'ok', hidden: false, deleted: false, pinned: false, reactions: [], comments: 0, pending: false, ...extra,
});
const range = (a: number, b: number) => Array.from({ length: b - a }, (_, i) => post(a + i));
const page = (items: FeedItemView[], extra: Partial<FeedPageView> = {}): FeedPageView => ({ items, asked: false, ...extra });

describe('the feed of a channel', () => {
  beforeEach(() => feedStore.reset());

  it('is kept by its channel and section', () => {
    feedStore.merge(feedKey('c'), page(range(0, 3)));
    feedStore.merge(feedKey('d'), page(range(5, 6)));
    feedStore.merge(feedKey('c', 's1'), page(range(10, 11)));
    expect(feedStore.shown(feedKey('c')).map((x) => x.text)).toEqual(['post 0', 'post 1', 'post 2']);
    expect(feedStore.shown(feedKey('d'))).toHaveLength(1);
    expect(feedStore.shown(feedKey('c', 's1'))[0].text).toBe('post 10');
    expect(feedStore.list(feedKey('c'))?.loaded).toBe(true);
    expect(feedStore.list(feedKey('e'))).toBeNull();
  });

  it('keeps the objects of items that did not change when read again', () => {
    const k = feedKey('c');
    feedStore.merge(k, page(range(0, 3)));
    const before = feedStore.shown(k);
    feedStore.merge(k, page([post(0), post(1, { reactions: [{ emoji: '👍', count: 1, mine: false }] }), post(2), post(3)]));
    const after = feedStore.shown(k);
    expect(after[0]).toBe(before[0]);
    expect(after[1]).not.toBe(before[1]);
    expect(after[2]).toBe(before[2]);
    expect(after.map((x) => x.text)).toEqual(['post 0', 'post 1', 'post 2', 'post 3']);
  });

  it('does not change at all when nothing changed', () => {
    const k = feedKey('c');
    feedStore.merge(k, page(range(0, 3)));
    const lists = feedStore.lists;
    feedStore.merge(k, page(range(0, 3)));
    expect(feedStore.lists).toBe(lists);
  });

  it('keeps what was read above the newest page and its cursor', () => {
    const k = feedKey('c');
    feedStore.merge(k, page(range(20, 30), { next: { created_at: 1020, id: 'p020' } }));
    return feedStore.earlier(k, async () => page(range(10, 20), { next: { created_at: 1010, id: 'p010' } })).then(() => {
      feedStore.merge(k, page(range(21, 31), { next: { created_at: 1021, id: 'p021' } }));
      const l = feedStore.list(k);
      expect(l?.items[0].text).toBe('post 10');
      expect(l?.items.at(-1)?.text).toBe('post 30');
      expect(l?.next).toEqual({ created_at: 1010, id: 'p010' });
    });
  });

  it('reads once more after a burst, never at once', async () => {
    const k = feedKey('c');
    let calls = 0;
    let release: () => void = () => {};
    const read = vi.fn(() => {
      calls += 1;
      return calls === 1 ? new Promise<FeedPageView>((r) => (release = () => r(page(range(0, 1))))) : Promise.resolve(page(range(0, 2)));
    });
    const first = feedStore.refresh(k, read);
    void feedStore.refresh(k, read);
    void feedStore.refresh(k, read);
    void feedStore.refresh(k, read);
    expect(read).toHaveBeenCalledTimes(1);
    release();
    await first;
    expect(read).toHaveBeenCalledTimes(2);
    expect(feedStore.shown(k)).toHaveLength(2);
  });

  it('says the relays were asked only of an empty list', () => {
    feedStore.merge(feedKey('c'), page([], { asked: true }));
    expect(feedStore.list(feedKey('c'))?.asked).toBe(true);
    feedStore.merge(feedKey('d'), page(range(0, 1), { asked: true }));
    expect(feedStore.list(feedKey('d'))?.asked).toBe(false);
  });

  it('keeps its newest items and where it was left', () => {
    const k = feedKey('c');
    feedStore.enter(k);
    feedStore.merge(k, page(range(0, KEEP_ITEMS + 20)));
    feedStore.leave(k, null);
    const l = feedStore.list(k);
    expect(l?.items).toHaveLength(KEEP_ITEMS);
    expect(l?.items.at(-1)?.text).toBe(`post ${KEEP_ITEMS + 19}`);
    expect(l?.next).toEqual({ created_at: l?.items[0].created_at, id: l?.items[0].id });
    expect(l?.anchor).toBeNull();
    // Left in the middle: all of it stays, with the place.
    feedStore.enter(k);
    feedStore.merge(k, page(range(0, KEEP_ITEMS + 20)));
    feedStore.leave(k, 480);
    expect(feedStore.list(k)?.anchor).toBe(480);
  });

  it('left as it was, is not written again: no list read from the store is drawn anew', () => {
    const k = feedKey('c', 's1');
    feedStore.enter(k);
    feedStore.merge(k, page(range(0, 5)));
    const lists = feedStore.lists;
    feedStore.leave(k, null);
    expect(feedStore.lists).toBe(lists);
    feedStore.enter(k);
    feedStore.leave(k, 200);
    expect(feedStore.lists).not.toBe(lists);
    expect(feedStore.list(k)?.anchor).toBe(200);
  });

  it('names the section of its key', () => {
    expect(scopeOfKey(feedKey('c'))).toBeNull();
    expect(scopeOfKey(feedKey('c', 's1'))).toBe('s1');
    expect(scopeOfKey(feedKey('c', 'everyone'))).toBe('everyone');
  });

  it('keeps the threads of posts apart from the feed', () => {
    feedStore.merge(threadKey('c', 'p001'), page([post(50, { post_id: 'p001' })]));
    expect(feedStore.shown(feedKey('c'))).toEqual([]);
    expect(feedStore.shown(threadKey('c', 'p001'))).toHaveLength(1);
  });

  it('forgets every list of a channel gone', () => {
    feedStore.merge(feedKey('c'), page(range(0, 1)));
    feedStore.merge(threadKey('c', 'p000'), page([post(5, { post_id: 'p000' })]));
    feedStore.merge(feedKey('d'), page(range(0, 1)));
    feedStore.dropChannel('c');
    expect(feedStore.list(feedKey('c'))).toBeNull();
    expect(feedStore.list(threadKey('c', 'p000'))).toBeNull();
    expect(feedStore.list(feedKey('d'))).not.toBeNull();
  });
});

describe('what I write', () => {
  beforeEach(() => feedStore.reset());
  const k = feedKey('c');

  it('stands in the feed at once, on its way until the runtime tells of it', () => {
    feedStore.merge(k, page(range(0, 2)));
    feedStore.addLocal(k, post(9, { id: 'local:1', pending: true }));
    expect(feedStore.shown(k).map((x) => x.id)).toEqual(['p000', 'p001', 'local:1']);
    expect(feedStore.isPending(feedStore.shown(k)[2])).toBe(true);
    feedStore.settle(k, 'local:1', 'p009');
    const mine = feedStore.shown(k)[2];
    expect(mine.id).toBe('p009');
    expect(feedStore.isPending({ id: 'p009' })).toBe(true);
    feedStore.heardOf('p009');
    expect(feedStore.isPending({ id: 'p009' })).toBe(false);
    // Read back: shown once, as read.
    feedStore.merge(k, page([...range(0, 2), post(9)]));
    expect(feedStore.shown(k).map((x) => x.id)).toEqual(['p000', 'p001', 'p009']);
    expect(feedStore.local[k]).toEqual([]);
  });

  it('is not on its way when the runtime told of it before the command answered', () => {
    feedStore.addLocal(k, post(9, { id: 'local:2' }));
    feedStore.heardOf('p009');
    feedStore.settle(k, 'local:2', 'p009');
    expect(feedStore.isPending({ id: 'p009' })).toBe(false);
  });

  it('is marked as on its way while the outbox holds it', () => {
    expect(feedStore.isPending(post(1, { pending: true }))).toBe(true);
    feedStore.outbox({ ref_id: 'p003', state: 'queued' });
    expect(feedStore.isPending({ id: 'p003' })).toBe(true);
    feedStore.outbox({ ref_id: 'p003', state: 'failed', error: 'no relay' });
    expect(feedStore.isPending({ id: 'p003' })).toBe(false);
    expect(feedStore.failure('p003')).toBe('no relay');
    feedStore.outbox({ ref_id: 'p003', state: 'queued' });
    expect(feedStore.failure('p003')).toBeNull();
    feedStore.outbox({ ref_id: 'p003', state: 'sent' });
    expect(feedStore.isPending({ id: 'p003' })).toBe(false);
  });

  it('is not on its way once a read says the outbox holds nothing of it, whatever came before', () => {
    // A post sealed again goes out twice: the `queued` of the second can be
    // heard after the `sent` of the first; the clock stayed for good.
    feedStore.outbox({ ref_id: 'p004', state: 'sent' });
    feedStore.outbox({ ref_id: 'p004', state: 'queued' });
    expect(feedStore.isPending({ id: 'p004' })).toBe(true);
    feedStore.merge(k, page([post(4, { pending: true })]));
    // Still in the outbox: still on its way.
    expect(feedStore.isPending({ id: 'p004' })).toBe(true);
    feedStore.merge(k, page([post(4, { pending: false })]));
    expect(feedStore.isPending({ id: 'p004' })).toBe(false);
  });

  it('goes when the command refuses it', () => {
    feedStore.addLocal(k, post(9, { id: 'local:3' }));
    feedStore.dropLocal(k, 'local:3');
    expect(feedStore.shown(k)).toEqual([]);
  });

  it('counts a comment of mine under its post at once', () => {
    feedStore.merge(k, page([post(1, { comments: 2 })]));
    feedStore.bumpComments('c', 'p001', 1);
    expect(feedStore.shown(k)[0].comments).toBe(3);
    feedStore.bumpComments('c', 'p001', -1);
    expect(feedStore.shown(k)[0].comments).toBe(2);
  });

  it('toggles a reaction on the tap and keeps it once sent', async () => {
    feedStore.merge(k, page([post(1, { reactions: [{ emoji: '👍', count: 2, mine: false }] })]));
    let done: () => void = () => {};
    const sent = vi.fn((set: boolean) => new Promise<void>((r) => { expect(set).toBe(true); done = r; }));
    const run = feedStore.react('c', feedStore.shown(k)[0], '👍', sent);
    expect(feedStore.shown(k)[0].reactions).toEqual([{ emoji: '👍', count: 3, mine: true }]);
    // A read meanwhile (of before the command) does not take the tap back.
    feedStore.merge(k, page([post(1, { reactions: [{ emoji: '👍', count: 2, mine: false }] })]));
    expect(feedStore.shown(k)[0].reactions[0]).toMatchObject({ count: 3, mine: true });
    done();
    await run;
    expect(feedStore.reacting).toEqual({});
    expect(feedStore.shown(k)[0].reactions[0]).toMatchObject({ count: 3, mine: true });
  });

  it('takes a reaction back when it is refused', async () => {
    feedStore.merge(k, page([post(1, { reactions: [{ emoji: '🔥', count: 1, mine: true }] })]));
    const refused = feedStore.react('c', feedStore.shown(k)[0], '🔥', async (set) => {
      expect(set).toBe(false);
      expect(feedStore.shown(k)[0].reactions).toEqual([]);
      throw new Error('not permitted');
    });
    await expect(refused).rejects.toThrow('not permitted');
    expect(feedStore.shown(k)[0].reactions).toEqual([{ emoji: '🔥', count: 1, mine: true }]);
  });
});

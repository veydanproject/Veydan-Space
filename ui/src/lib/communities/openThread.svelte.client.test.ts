// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The comments open beside the feed follow their post: a reaction on its
// way to the post settles at once (it once wrote the thread again and
// again until Svelte stopped the window), a new count shows, a removal
// closes the panel.

import { flushSync } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FeedItemView, FeedPageView } from './api';
import { feedKey, feedStore } from './feedStore.svelte';
import { OpenThread } from './openThread.svelte';

const post = (extra: Partial<FeedItemView> = {}): FeedItemView => ({
  id: 'p1', community_id: 'c', post_id: '', author: 'a', created_at: 1000,
  text: 'post', review: 'ok', hidden: false, deleted: false, pinned: false, reactions: [], comments: 0, pending: false, ...extra,
});
const page = (items: FeedItemView[]): FeedPageView => ({ items, asked: false });
const k = feedKey('c');

let stop = () => {};
function open(ongone = () => {}) {
  let thread!: OpenThread;
  stop = $effect.root(() => {
    // As the feed holds it (ChannelFeed's `items`).
    const items = $derived(feedStore.shown(k));
    thread = new OpenThread(() => items, ongone);
  });
  flushSync();
  thread.post = feedStore.shown(k)[0];
  flushSync();
  return thread;
}

beforeEach(() => {
  feedStore.reset();
  feedStore.merge(k, page([post()]));
});
afterEach(() => stop());

describe('the post of open comments', () => {
  it('takes a reaction on its way without writing itself again and again', () => {
    const thread = open();
    // The command never answers: the reaction stays on its way.
    void feedStore.react('c', thread.post!, '❤️', () => new Promise(() => {}));
    expect(() => flushSync()).not.toThrow();
    expect(thread.post?.reactions).toEqual([{ emoji: '❤️', count: 1, mine: true }]);
  });

  it('follows the post read again', () => {
    const thread = open();
    feedStore.merge(k, page([post({ comments: 3 })]));
    flushSync();
    expect(thread.post?.comments).toBe(3);
  });

  it('closes on a removal', () => {
    const ongone = vi.fn();
    const thread = open(ongone);
    feedStore.merge(k, page([post({ deleted: true })]));
    flushSync();
    expect(thread.post).toBeNull();
    expect(ongone).toHaveBeenCalledOnce();
  });
});

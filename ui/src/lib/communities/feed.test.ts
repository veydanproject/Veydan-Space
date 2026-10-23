// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { FeedItemView } from './api';
import { withEarlier, withNewest } from './feed';

const post = (n: number): FeedItemView => ({
  id: `p${String(n).padStart(2, '0')}`, community_id: 'c', post_id: '', author: 'a', created_at: 1000 + n,
  text: `post ${n}`, review: 'ok', hidden: false, deleted: false, pinned: false, reactions: [], comments: 0, pending: false,
});
const texts = (xs: FeedItemView[]) => xs.map((x) => x.text);
const range = (a: number, b: number) => Array.from({ length: b - a }, (_, i) => post(a + i));

describe('the feed reads as a chat', () => {
  it('shows the newest page oldest at the top, the newest at the bottom', () => {
    const { items } = withNewest([], range(20, 30));
    expect(texts(items)).toEqual(range(20, 30).map((p) => p.text));
    expect(items.at(-1)?.text).toBe('post 29');
  });

  it('puts a page of the newest first in order too', () => {
    expect(texts(withNewest([], range(20, 30).reverse()).items)).toEqual(range(20, 30).map((p) => p.text));
  });

  it('puts earlier pages above what was read', () => {
    let items = withNewest([], range(20, 30)).items;
    items = withEarlier(items, range(10, 20));
    items = withEarlier(items, range(0, 10));
    expect(texts(items)).toEqual(range(0, 30).map((p) => p.text));
  });

  it('keeps what was read above when the newest page is read again', () => {
    let items = withNewest([], range(20, 30)).items;
    items = withEarlier(items, range(10, 20));
    const again = withNewest(items, range(21, 31));
    expect(again.older).toBe(true);
    expect(texts(again.items)).toEqual(range(10, 31).map((p) => p.text));
  });
});

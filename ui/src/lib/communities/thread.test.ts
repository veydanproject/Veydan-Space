// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { FeedItemView } from './api';
import { threadOrder } from './thread';

const c = (id: string, at: number, parent?: string): FeedItemView => ({
  id, community_id: 'c', post_id: 'p', parent_id: parent, author: 'a', created_at: at, review: 'ok', hidden: false, deleted: false,
  pinned: false, reactions: [], comments: 0, pending: false,
});
const order = (list: FeedItemView[]) => threadOrder(list).map((l) => `${l.item.id}:${l.depth}`);

describe('the order of the comments of a post', () => {
  it('puts each answer under the comment it answers, oldest first at every level', () => {
    expect(order([c('a', 1), c('b', 2), c('a1', 3, 'a'), c('a2', 5, 'a'), c('a1x', 4, 'a1'), c('b1', 6, 'b')])).toEqual([
      'a:0', 'a1:1', 'a1x:2', 'a2:1', 'b:0', 'b1:1',
    ]);
  });

  it('shows an answer to a comment not on the page as a comment of its own', () => {
    expect(order([c('x', 2, 'gone'), c('y', 1)])).toEqual(['y:0', 'x:0']);
  });

  it('shows every comment of a loop of answers once', () => {
    expect(order([c('a', 1, 'b'), c('b', 2, 'a')])).toEqual(['a:0', 'b:1']);
  });
});

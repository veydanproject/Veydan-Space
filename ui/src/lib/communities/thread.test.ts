// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { FeedItemView } from './api';
import { arrived, shownOf, threadOrder } from './thread';

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

// They end the «печатает…» of their authors (ThreadView).
describe('the comments that came while a thread was shown', () => {
  const by = (id: string, at: number, author: string): FeedItemView => ({ ...c(id, at), author });

  it('are the new ones, not a page of earlier ones nor mine on their way', () => {
    const before = shownOf([by('m', 10, 'x'), by('n', 20, 'y')]);
    const now = [by('e', 5, 'old'), by('m', 10, 'x'), by('n', 20, 'y'), by('o', 20, 'eva'), by('q', 31, 'artem'), by('local:1', 40, 'me')];
    expect(arrived(before, now).map((x) => `${x.id}:${x.author}`)).toEqual(['o:eva', 'q:artem']);
  });

  it('are all of them when nothing was shown before', () => {
    expect(arrived(shownOf([]), [by('a', 1, 'eva')]).map((x) => x.author)).toEqual(['eva']);
  });
});

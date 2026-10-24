// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The order of the comments of a post: the comments to the post oldest
// first, each followed by the answers to it (and the answers to those),
// oldest first in their turn. An answer to a comment not on the page (an
// earlier page, or one removed) stands as a comment of its own. A page
// indents the answers by their depth, three steps at most.

import type { FeedItemView } from './api';

export interface ThreadLine {
  item: FeedItemView;
  /** 0 for a comment to the post, 1 for an answer to one, and so on. */
  depth: number;
}

export function threadOrder(comments: readonly FeedItemView[]): ThreadLine[] {
  const here = new Set(comments.map((c) => c.id));
  const children = new Map<string, FeedItemView[]>();
  const roots: FeedItemView[] = [];
  for (const c of [...comments].sort((a, b) => a.created_at - b.created_at || a.id.localeCompare(b.id))) {
    const parent = c.parent_id && here.has(c.parent_id) && c.parent_id !== c.id ? c.parent_id : null;
    if (parent) {
      const list = children.get(parent) ?? [];
      list.push(c);
      children.set(parent, list);
    } else {
      roots.push(c);
    }
  }
  const out: ThreadLine[] = [];
  const seen = new Set<string>();
  const walk = (c: FeedItemView, depth: number) => {
    if (seen.has(c.id)) return;
    seen.add(c.id);
    out.push({ item: c, depth });
    for (const k of children.get(c.id) ?? []) walk(k, depth + 1);
  };
  for (const r of roots) walk(r, 0);
  // A loop of answers (a page of a newer core) still shows every comment.
  for (const c of comments) if (!seen.has(c.id)) walk(c, 0);
  return out;
}

/** What a page of a thread showed: the ids of its comments and the date of the newest. */
export interface Shown {
  ids: Set<string>;
  newest: number;
}

export function shownOf(comments: readonly FeedItemView[]): Shown {
  return { ids: new Set(comments.map((c) => c.id)), newest: comments.reduce((n, c) => Math.max(n, c.created_at), 0) };
}

/**
 * The comments that came since `before` was shown: not among its ids, and
 * not older than its newest (those are a page of earlier ones). Mine still
 * on their way (`local:`) were written here, they did not come.
 */
export function arrived(before: Shown, comments: readonly FeedItemView[]): FeedItemView[] {
  return comments.filter((c) => !before.ids.has(c.id) && c.created_at >= before.newest && !c.id.startsWith('local:'));
}

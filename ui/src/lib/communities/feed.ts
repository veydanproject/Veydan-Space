// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The feed of a channel and the comments of a post read as a chat: the
// oldest at the top, the newest at the bottom, and earlier pages above
// what was read. A page of `channel_history` and `channel_thread` comes
// oldest first; these two put it into what is shown.

import type { FeedItemView } from './api';

/** By date, then by id: the order of the pages' cursor. */
export function byTime(a: FeedItemView, b: FeedItemView): number {
  return a.created_at - b.created_at || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

/**
 * The newest page read again: it replaces what it covers, what was read
 * above it stays. `older`: whether anything of what was read stayed
 * above (then the cursor of the earlier pages stays as it was).
 */
export function withNewest(shown: readonly FeedItemView[], page: readonly FeedItemView[]): { items: FeedItemView[]; older: boolean } {
  const sorted = [...page].sort(byTime);
  const first = sorted[0]?.created_at ?? Infinity;
  const ids = new Set(sorted.map((x) => x.id));
  const above = shown.filter((x) => x.created_at < first && !ids.has(x.id));
  return { items: [...above, ...sorted], older: above.length > 0 };
}

/** A page from before what was read, put above it. */
export function withEarlier(shown: readonly FeedItemView[], page: readonly FeedItemView[]): FeedItemView[] {
  const ids = new Set(shown.map((x) => x.id));
  return [...page.filter((x) => !ids.has(x.id)).sort(byTime), ...shown];
}

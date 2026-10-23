// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// My reactions are those the runtime marks (`ReactionCount.mine`, read from
// the database, so one set on another device of mine is mine here too): a
// tap on a chip of mine takes it back, any other sets it. The tap shows at
// once (feedStore.react); the next read of the page says how it stands.

import type { FeedItemView } from './api';

/** Have I set `emoji` on this post or comment? */
export function isMine(item: Pick<FeedItemView, 'reactions'>, emoji: string): boolean {
  return item.reactions.some((r) => r.emoji === emoji && r.mine);
}

/** The quick reactions over a menu of a post: the usual six of the messenger. */
export const QUICK = ['❤️', '👍', '😂', '😮', '😢', '🔥'] as const;

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { MessageStatus, MessengerMessage } from '../api';

/** A message just written is shown as sent: the outbox holds it and keeps
 *  trying for an hour. Not out after this long, it shows a clock. */
export const SEND_PATIENCE_SECS = 15;

export type ShownStatus = MessageStatus | 'waiting' | 'delivered' | 'read';

type Timed = Pick<MessengerMessage, 'status' | 'created_at' | 'queued_at'> &
  Partial<Pick<MessengerMessage, 'delivered_at' | 'read_at'>>;

/** When a queued message stops being shown as sent: counted from when it
 *  went to the outbox (or back to it by a retry), not from its own time. */
export function lateAt(m: Timed): number {
  return (m.queued_at ?? m.created_at) + SEND_PATIENCE_SECS;
}

/** What the bubble shows: `queued` is a promise for a while, then
 *  `waiting`; a sent message the peer has, or has read, says so;
 *  everything else is what it is. */
export function shownStatus(m: Timed, nowSecs: number): ShownStatus {
  if (m.status === 'sent') return m.read_at ? 'read' : m.delivered_at ? 'delivered' : 'sent';
  if (m.status !== 'queued') return m.status;
  return nowSecs < lateAt(m) ? 'sent' : 'waiting';
}

// From worst to best. A paused upload waits like a late one; a queued part
// still in its moment of patience is as good as sent.
const RANK: Partial<Record<ShownStatus, number>> = {
  failed: 0, uploading: 1, waiting: 2, paused: 2, queued: 3, sent: 3, delivered: 4, read: 5,
};
const NAME = ['failed', 'uploading', 'waiting', 'sent', 'delivered', 'read'] as const;

/** What an album of several messages shows: the worst of its parts, so it
 *  is read only when every part is, delivered only when every part is at
 *  least delivered. Nothing to judge is sent. */
export function worstOf(statuses: Iterable<ShownStatus>): (typeof NAME)[number] {
  let worst = 3;
  let any = false;
  for (const s of statuses) {
    const r = RANK[s] ?? 3;
    worst = any ? Math.min(worst, r) : r;
    any = true;
  }
  return NAME[worst];
}

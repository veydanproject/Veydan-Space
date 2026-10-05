// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { MessageStatus, MessengerMessage } from '../api';

/** A message just written is shown as sent: the outbox holds it and keeps
 *  trying for an hour. Not out after this long, it shows a clock. */
export const SEND_PATIENCE_SECS = 15;

export type ShownStatus = MessageStatus | 'waiting';

type Timed = Pick<MessengerMessage, 'status' | 'created_at' | 'queued_at'>;

/** When a queued message stops being shown as sent: counted from when it
 *  went to the outbox (or back to it by a retry), not from its own time. */
export function lateAt(m: Timed): number {
  return (m.queued_at ?? m.created_at) + SEND_PATIENCE_SECS;
}

/** What the bubble shows: `queued` is a promise for a while, then
 *  `waiting`; everything else is what it is. */
export function shownStatus(m: Timed, nowSecs: number): ShownStatus {
  if (m.status !== 'queued') return m.status;
  return nowSecs < lateAt(m) ? 'sent' : 'waiting';
}

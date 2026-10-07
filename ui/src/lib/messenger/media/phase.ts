// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Where the attachment of one message is, in one word, as its bubble shows it.

import type { MessengerTransferProgress, TransferStatus } from '../api';

export type BubblePhase =
  | 'uploading'
  | 'upload_paused'
  | 'upload_failed'
  | 'here'
  | 'downloading'
  | 'download_paused'
  | 'download_failed'
  | 'remote';

/** A transfer under way: queued, running, or waiting for its next attempt. */
const UNDER_WAY: TransferStatus[] = ['queued', 'running', 'waiting_retry'];

/**
 * The phase of a message's attachment. `out`, `status` and `id` are the
 * message's; `here` whether the file is on this device; `live` the last
 * event of its transfer.
 */
export function bubblePhase(
  m: { out: boolean; status: string; id: string },
  here: boolean,
  live: Pick<MessengerTransferProgress, 'direction' | 'status'> | null,
): BubblePhase {
  if (m.out && (m.status === 'uploading' || (live?.status === 'running' && live.direction === 'up'))) return 'uploading';
  if (m.out && m.status === 'paused') return 'upload_paused';
  if (m.out && m.status === 'failed' && m.id.startsWith('local:')) return 'upload_failed';
  if (here) return 'here';
  if (live?.direction === 'down' && UNDER_WAY.includes(live.status)) return 'downloading';
  if (live?.direction === 'down' && live.status === 'paused') return 'download_paused';
  if (live?.direction === 'down' && live.status === 'failed') return 'download_failed';
  return 'remote';
}

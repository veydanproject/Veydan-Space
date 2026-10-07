// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { TransferStatus } from '../api';
import { bubblePhase } from './phase';

const theirs = { out: false, status: 'received', id: 'm1' };
const down = (status: TransferStatus) => ({ direction: 'down' as const, status });

describe('the phase of an attachment', () => {
  it('shows a download under way, also while it waits for its next attempt', () => {
    for (const status of ['queued', 'running', 'waiting_retry'] as const) {
      expect(bubblePhase(theirs, false, down(status)), status).toBe('downloading');
    }
    expect(bubblePhase(theirs, false, down('paused'))).toBe('download_paused');
    expect(bubblePhase(theirs, false, down('failed'))).toBe('download_failed');
    expect(bubblePhase(theirs, false, null)).toBe('remote');
    expect(bubblePhase(theirs, true, down('running'))).toBe('here');
  });

  it('follows the placeholder of an upload', () => {
    const mine = (status: string) => ({ out: true, status, id: 'local:1' });
    expect(bubblePhase(mine('uploading'), true, { direction: 'up', status: 'waiting_retry' })).toBe('uploading');
    expect(bubblePhase(mine('paused'), true, null)).toBe('upload_paused');
    expect(bubblePhase(mine('failed'), true, null)).toBe('upload_failed');
    expect(bubblePhase({ out: true, status: 'sent', id: 'e1' }, true, null)).toBe('here');
  });
});

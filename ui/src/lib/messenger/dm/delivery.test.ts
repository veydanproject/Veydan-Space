// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { lateAt, SEND_PATIENCE_SECS, shownStatus } from './delivery';

describe('shownStatus', () => {
  it('shows a queued message as sent for a while, then as waiting', () => {
    const m = { status: 'queued' as const, created_at: 1_000, queued_at: 1_000 };
    expect(shownStatus(m, 1_000)).toBe('sent');
    expect(shownStatus(m, 1_000 + SEND_PATIENCE_SECS - 1)).toBe('sent');
    expect(shownStatus(m, 1_000 + SEND_PATIENCE_SECS)).toBe('waiting');
  });

  it('counts from the outbox, not from the message time', () => {
    // A finished upload keeps the time it started; a retry is a new start.
    const m = { status: 'queued' as const, created_at: 100, queued_at: 5_000 };
    expect(lateAt(m)).toBe(5_000 + SEND_PATIENCE_SECS);
    expect(shownStatus(m, 5_001)).toBe('sent');
    expect(shownStatus({ ...m, queued_at: null }, 5_001)).toBe('waiting');
  });

  it('leaves every other status as it is', () => {
    for (const status of ['sent', 'failed', 'received', 'uploading', 'paused'] as const) {
      expect(shownStatus({ status, created_at: 0, queued_at: null }, 10_000)).toBe(status);
    }
  });
});

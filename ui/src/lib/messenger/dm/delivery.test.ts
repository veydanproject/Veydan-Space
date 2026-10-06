// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { lateAt, SEND_PATIENCE_SECS, shownStatus, worstOf } from './delivery';

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

  it('shows a sent message the peer has as delivered, and one it read as read', () => {
    const m = { status: 'sent' as const, created_at: 0, queued_at: null, delivered_at: null, read_at: null };
    expect(shownStatus(m, 10)).toBe('sent');
    expect(shownStatus({ ...m, delivered_at: 5 }, 10)).toBe('delivered');
    expect(shownStatus({ ...m, delivered_at: 5, read_at: 7 }, 10)).toBe('read');
    // A read implies delivery, even when the delivery note never came.
    expect(shownStatus({ ...m, read_at: 7 }, 10)).toBe('read');
  });

  it('takes no receipt for a message that is not sent', () => {
    const m = { created_at: 0, queued_at: 0, delivered_at: 5, read_at: 7 };
    expect(shownStatus({ ...m, status: 'failed' as const }, 10)).toBe('failed');
    expect(shownStatus({ ...m, status: 'queued' as const }, 1)).toBe('sent');
    expect(shownStatus({ ...m, status: 'queued' as const }, 100)).toBe('waiting');
  });
});

describe('worstOf', () => {
  it('picks the worst part in the order failed, uploading, waiting, sent, delivered, read', () => {
    expect(worstOf(['read', 'failed', 'uploading'])).toBe('failed');
    expect(worstOf(['read', 'uploading', 'waiting'])).toBe('uploading');
    expect(worstOf(['read', 'waiting', 'sent'])).toBe('waiting');
    expect(worstOf(['read', 'paused'])).toBe('waiting');
    expect(worstOf(['read', 'delivered', 'sent'])).toBe('sent');
    expect(worstOf(['read', 'queued'])).toBe('sent');
  });

  it('is read only when every part is read, delivered only when every part has at least that', () => {
    expect(worstOf(['read', 'read'])).toBe('read');
    expect(worstOf(['read', 'delivered'])).toBe('delivered');
    expect(worstOf(['delivered', 'delivered'])).toBe('delivered');
  });

  it('calls nothing sent', () => {
    expect(worstOf([])).toBe('sent');
  });
});

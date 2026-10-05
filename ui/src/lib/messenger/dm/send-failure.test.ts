// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it, vi } from 'vitest';
import { logSendFailure, sendFailureKey } from './send-failure';

describe('the reason of an unsent message', () => {
  it('says "no connection" for the transport errors of the backend', () => {
    expect(sendFailureKey('transport error: no relay connected')).toBe('msg_send_failed_offline');
    expect(sendFailureKey('no relay accepted')).toBe('msg_send_failed_offline');
    expect(sendFailureKey('offline')).toBe('msg_send_failed_offline');
  });

  it('says "no connection for an hour" for a message the outbox gave up', () => {
    expect(sendFailureKey('expired: transport error: no relay connected')).toBe('msg_send_failed_expired');
    expect(sendFailureKey('rejected by wss://r.example: blocked: spam')).toBe('msg_send_failed');
  });

  it('says "not sent" for anything else, never the raw text', () => {
    expect(sendFailureKey('storage error: disk full')).toBe('msg_send_failed');
    expect(sendFailureKey(null)).toBe('msg_send_failed');
    expect(sendFailureKey('')).toBe('msg_send_failed');
  });

  it('logs the raw text once per message', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    logSendFailure('a', 'transport error: no relay connected');
    logSendFailure('a', 'transport error: no relay connected');
    logSendFailure('b', null);
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn.mock.calls[0][0]).toContain('transport error: no relay connected');
    warn.mockRestore();
  });
});

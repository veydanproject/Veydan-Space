// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { MessengerRuntimeStatus } from '../api';
import { netState } from './net-state';

const rt = (patch: Partial<MessengerRuntimeStatus>) =>
  ({ session_active: true, silent_mode: false, relays_connected: 0, relays_total: 2, link: 'ok', ...patch }) as MessengerRuntimeStatus;

describe('netState', () => {
  it('promises a connection until the runtime takes it back', () => {
    expect(netState(rt({ relays_connected: 0, link: 'ok' }))).toBe('on');
    expect(netState(rt({ link: 'waiting' }))).toBe('connecting');
    expect(netState(rt({ link: 'lost' }))).toBe('lost');
  });

  it('says why it is quiet before anything else', () => {
    expect(netState(null)).toBe('off');
    expect(netState(rt({ silent_mode: true, link: 'lost' }))).toBe('silent');
    expect(netState(rt({ session_active: false, link: 'lost' }))).toBe('locked');
  });
});

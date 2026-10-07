// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The words under the peer's name, and what a screen reader is told of
// them: the phase as it changes, never the clock that ticks every second.

import { describe, expect, it } from 'vitest';
import type { CallView } from '../generated/calls';
import { liveText, statusText } from './status';

const tr = (key: string) => key;

function view(over: Partial<CallView> = {}): CallView {
  return {
    call_id: 'c1', peer: 'ab'.repeat(32), chat_id: 'dm:x', direction: 'out', media: 'audio', phase: 'outgoing',
    muted: false, started_at: 1000, nodes: [], video_local: false, video_screen: false, video_remote: false, ...over,
  };
}

describe('what a screen reader is told of a call', () => {
  it('tells the phases', () => {
    expect(liveText(view(), null, tr)).toBe('msg_call_phase_outgoing');
    expect(liveText(view({ phase: 'connecting', answered_at: 1003 }), null, tr)).toBe('msg_call_phase_reconnecting');
  });

  it('does not tell the clock that the screen shows while the call talks', () => {
    const talking = view({ phase: 'active', answered_at: 1003 });
    expect(statusText(talking, null, 1010, tr)).toBe('0:07');
    expect(liveText(talking, null, tr)).toBe('');
    expect(statusText(talking, null, 1011, tr), 'the screen ticks').toBe('0:08');
    expect(liveText(talking, null, tr), 'the live region does not').toBe('');
  });

  it('tells how the call ended, once', () => {
    const ended = { call: view({ phase: 'ended' }), outcome: 'ended' as const, duration: 65 };
    expect(liveText(null, ended, tr)).toBe('msg_call_ended · 1:05');
  });
});

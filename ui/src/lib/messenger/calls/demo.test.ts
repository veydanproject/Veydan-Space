// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The video of the browser preview's calls goes as the runtime's does: one
// frame on its way at a time, the next only once the page acknowledged it
// (`messenger_call_video_ack`).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MessengerChat, MessengerMessage } from '../api';
import { demoCallMocks } from './demo';
import { frameSeq, parseFrame } from './video';

const ALICE = '1a'.repeat(32);

function mocks() {
  const lines: MessengerMessage[] = [];
  return demoCallMocks({
    demo: false,
    emit: () => {},
    chat: (peer) => ({ id: `chat:${peer}`, peer_pubkey: peer, mode: 'full_chat' }) as unknown as MessengerChat,
    lines: () => lines,
    touch: () => {},
  });
}

describe('the video of the demo', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('sends the next frame only after the last one was acknowledged', () => {
    const m = mocks();
    m.messenger_call_start({ peer: ALICE, media: 'video' });
    const got: ArrayBuffer[] = [];
    const id = m.messenger_call_video_subscribe({ track: 'local', channel: { onmessage: (d: ArrayBuffer) => got.push(d) } }) as number;

    vi.advanceTimersByTime(1000);
    expect(got.length, 'one frame, then it waits').toBe(1);
    expect(frameSeq(got[0])).toBe(0);
    expect(parseFrame(got[0])).not.toBeNull();

    // Another subscription's ack, or none: still nothing.
    m.messenger_call_video_ack({ id: id + 1, seq: 0 });
    vi.advanceTimersByTime(500);
    expect(got.length).toBe(1);

    m.messenger_call_video_ack({ id, seq: 0 });
    vi.advanceTimersByTime(1000);
    expect(got.map(frameSeq)).toEqual([0, 1]);

    // An ack of an older frame does not free the way.
    m.messenger_call_video_ack({ id, seq: 0 });
    vi.advanceTimersByTime(500);
    expect(got.length).toBe(2);

    m.messenger_call_video_ack({ id, seq: 1 });
    vi.advanceTimersByTime(100);
    expect(got.map(frameSeq)).toEqual([0, 1, 2]);
  });

  it('sends the last message when the call ends, acknowledged or not', () => {
    const m = mocks();
    const view = m.messenger_call_start({ peer: ALICE, media: 'video' }) as { call_id: string };
    const got: ArrayBuffer[] = [];
    m.messenger_call_video_subscribe({ track: 'local', channel: { onmessage: (d: ArrayBuffer) => got.push(d) } });
    vi.advanceTimersByTime(500);
    expect(got.length).toBe(1);
    m.messenger_call_end({ callId: view.call_id });
    expect(got.length).toBe(2);
    expect(parseFrame(got[1])).toBe('end');
  });
});

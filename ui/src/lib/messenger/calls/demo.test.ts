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
const BORIS = '2b'.repeat(32);
const ME = 'ab'.repeat(32);
const GROUP = '7a'.repeat(64).slice(0, 64);

function mocks() {
  const lines: MessengerMessage[] = [];
  return demoCallMocks({
    demo: false,
    emit: () => {},
    chat: (peer) => ({ id: `chat:${peer}`, peer_pubkey: peer, mode: 'full_chat' }) as unknown as MessengerChat,
    lines: () => lines,
    touch: () => {},
    me: () => ME,
    members: (groupId) => (groupId === GROUP ? [ME, ALICE, BORIS] : []),
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

describe('the group calls of the demo', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('fills a room I started, one call of either kind at a time', () => {
    const m = mocks();
    const view = m.messenger_group_call_start({ groupId: GROUP, media: 'audio' }) as { phase: string };
    expect(view.phase).toBe('starting');
    expect(() => m.messenger_call_start({ peer: ALICE })).toThrow();
    expect(() => m.messenger_group_call_start({ groupId: GROUP })).toThrow();
    vi.advanceTimersByTime(9000);
    const s = m.messenger_group_call_get_state({ groupId: GROUP }) as { call: { phase: string; participants: { verified: boolean; video_mid?: string }[] }; announced: { participants: string[] } };
    expect(s.call.phase).toBe('in_room');
    expect(s.call.participants.length).toBe(3);
    expect(s.call.participants.every((p) => p.verified)).toBe(true);
    expect(s.announced.participants).toContain(ME);
  });

  it('sends the video of a seat at the size of the layer asked for', () => {
    const m = mocks();
    m.messenger_group_call_start({ groupId: GROUP, media: 'video' });
    vi.advanceTimersByTime(9000);
    const s = m.messenger_group_call_get_state({}) as { call: { participants: { id: number; video_mid?: string }[] } };
    // Every seat has a video m-line, as the node gives them; the last one came with its camera on.
    expect(s.call.participants.filter((p) => p.id !== 1).every((p) => p.video_mid)).toBe(true);
    const seat = s.call.participants[s.call.participants.length - 1];
    m.messenger_group_call_set_layer({ participant: seat.id, rid: 'q' });
    const got: ArrayBuffer[] = [];
    const id = m.messenger_group_call_video_subscribe({ mid: seat.video_mid, channel: { onmessage: (d: ArrayBuffer) => got.push(d) } }) as number;
    vi.advanceTimersByTime(200);
    const f = parseFrame(got[0]);
    expect(f && f !== 'end' ? [f.width, f.height] : null).toEqual([320, 180]);
    // Acknowledged like the frames of a call of two; the end of the room ends them.
    m.messenger_call_video_ack({ id, seq: 0 });
    m.messenger_group_call_leave();
    expect(parseFrame(got[got.length - 1])).toBe('end');
  });
});

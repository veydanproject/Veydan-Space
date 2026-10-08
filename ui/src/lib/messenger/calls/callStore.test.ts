// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The store follows the runtime's events: one call at a time, a late word
// of a call that is over is no new call, a call missed while away has no
// screen to close, and a hang-up shows at once. My video: the camera on and
// off, the screen in its place and back, a video call taken without it.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CallView, GroupCallView } from '../generated/calls';

const calls = vi.hoisted(() => ({
  state: vi.fn(),
  start: vi.fn(),
  accept: vi.fn(),
  decline: vi.fn(async () => {}),
  end: vi.fn(async () => {}),
  mute: vi.fn(),
  setPolicy: vi.fn(),
  setNodes: vi.fn(),
  audioRoute: vi.fn(),
  setVideo: vi.fn(),
  switchCamera: vi.fn(),
  shareScreen: vi.fn(),
  screens: vi.fn(),
  cameras: vi.fn(),
  setVideoQuality: vi.fn(),
  setIncoming: vi.fn(),
}));
vi.mock('../api', () => ({ messengerApi: { calls } }));

const { callStore, CONFIRM_KEY, RING_TIMEOUT_SECS, gaveUp } = await import('./callStore.svelte');
const { groupCallStore } = await import('./groupCallStore.svelte');
const { endSound } = await import('./sounds');

function view(over: Partial<CallView> = {}): CallView {
  return {
    call_id: 'c1', peer: 'ab'.repeat(32), chat_id: `dm:${'ab'.repeat(32)}`, direction: 'in', media: 'audio', phase: 'incoming',
    muted: false, started_at: 1000, nodes: [], video_local: false, video_screen: false, video_remote: false, ...over,
  };
}

/** The room of a group call, as the group store hears of it (the module store hands every event to both stores). */
function room(phase: GroupCallView['phase']): GroupCallView {
  const me = 'ab'.repeat(32);
  return {
    call_id: 'g1', group_id: 'grp', chat_id: 'group:grp', phase, media: 'audio', muted: false, video_local: false,
    started_by: me, started_at: 900, joined_at: 901, node: '1.2.3.4:8443#' + 'cd'.repeat(32), participant: 1, epoch: 1,
    participants: [{ id: 1, npub: me, verified: true, speaking: false, audio: true, me: true }], kbps_per_participant: 0, max_participants: 0,
  };
}

describe('the call store', () => {
  beforeEach(() => {
    callStore.reset();
    groupCallStore.reset();
    vi.clearAllMocks();
  });

  it('says busy for a call that rings while I sit in the room of a group call', () => {
    groupCallStore.handleEvent({ name: 'group_call.state', payload: { call: room('in_room') } });
    callStore.handleEvent({ name: 'call.incoming', payload: { call: view(), busy_with_group: true } });
    expect(callStore.ringing).toBe(true);
    expect(callStore.busyWithGroup).toBe(true);
    // The word is for this call: another one, or its end, takes it away.
    callStore.handleEvent({ name: 'call.ended', payload: { call: view({ phase: 'ended' }), outcome: 'missed', duration_secs: null } });
    expect(callStore.busyWithGroup).toBe(false);
    callStore.handleEvent({ name: 'call.incoming', payload: { call: view({ call_id: 'c2' }) } });
    expect(callStore.busyWithGroup, 'a payload without the word is not busy').toBe(false);
    callStore.handleEvent({ name: 'call.incoming', payload: { call: view({ call_id: 'c3' }), busy_with_group: true } });
    expect(callStore.busyWithGroup).toBe(true);
    callStore.reset();
    expect(callStore.busyWithGroup).toBe(false);
  });

  /**
   * In the room of a Trio, Alice calls 1:1: "busy", Decline alone. I leave
   * the room to take her call while it still rings (the invitation lives
   * 45 s): the runtime would take `call_accept` now, and the screen once
   * kept showing "busy" to the end of the call. The word of the runtime
   * holds only while the room does.
   */
  it('takes "busy" away once I am out of the room while the call still rings', () => {
    groupCallStore.handleEvent({ name: 'group_call.state', payload: { call: room('in_room') } });
    callStore.handleEvent({ name: 'call.incoming', payload: { call: view(), busy_with_group: true } });
    expect(callStore.busyWithGroup).toBe(true);
    groupCallStore.handleEvent({ name: 'group_call.state', payload: { call: room('left') } });
    expect(groupCallStore.call).toBeNull();
    expect(callStore.ringing, 'the call rings on').toBe(true);
    expect(callStore.busyWithGroup, 'Answer is back').toBe(false);
    // The room that went for everybody says `ended` after `left`: still not busy.
    groupCallStore.handleEvent({ name: 'group_call.ended', payload: { call: { call_id: 'g1', group_id: 'grp' }, outcome: 'ended' } });
    expect(callStore.busyWithGroup).toBe(false);
  });

  it('takes a ringing call and its changes', () => {
    callStore.handleEvent({ name: 'call.incoming', payload: { call: view() } });
    expect(callStore.ringing).toBe(true);
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ phase: 'active', via: 'relay', answered_at: 1005 }) } });
    expect(callStore.call?.phase).toBe('active');
    expect(callStore.call?.via).toBe('relay');
    callStore.handleEvent({ name: 'call.stats', payload: { call_id: 'c1', stats: { rtt_ms: 48 } } });
    callStore.handleEvent({ name: 'call.level', payload: { call_id: 'c1', level: 3 } });
    callStore.handleEvent({ name: 'call.stats', payload: { call_id: 'other', stats: { rtt_ms: 999 } } });
    expect(callStore.stats?.rtt_ms).toBe(48);
    expect(callStore.level).toBe(1);
  });

  it('ends the call with the runtime\'s word, and does not bring it back after', () => {
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ phase: 'active', answered_at: 1005 }) } });
    callStore.handleEvent({ name: 'call.ended', payload: { call: view({ phase: 'ended' }), outcome: 'ended', duration_secs: 42 } });
    expect(callStore.call).toBeNull();
    expect(callStore.ended).toMatchObject({ outcome: 'ended', duration: 42 });
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ phase: 'active', answered_at: 1005 }) } });
    expect(callStore.call, 'a late state of the call that ended').toBeNull();
  });

  it('has no screen to close for a call missed while away', () => {
    callStore.handleEvent({ name: 'call.ended', payload: { call: view({ call_id: 'old' }), outcome: 'missed', duration_secs: null } });
    expect(callStore.ended).toBeNull();
  });

  it('keeps the call under way when another one ends', () => {
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ phase: 'active' }) } });
    callStore.handleEvent({ name: 'call.ended', payload: { call: view({ call_id: 'c2' }), outcome: 'busy', duration_secs: null } });
    expect(callStore.call?.call_id).toBe('c1');
  });

  it('closes the screen at once on a hang-up; the runtime\'s word follows', async () => {
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ direction: 'out', phase: 'outgoing' }) } });
    await callStore.hangUp();
    expect(calls.end).toHaveBeenCalledWith('c1');
    expect(callStore.call).toBeNull();
    expect(callStore.ended?.outcome).toBe('missed');
    callStore.handleEvent({ name: 'call.ended', payload: { call: view({ direction: 'out' }), outcome: 'busy', duration_secs: null } });
    expect(callStore.ended?.outcome).toBe('busy');
  });

  it('keeps a refusal for the screen', async () => {
    calls.start.mockRejectedValueOnce({ code: 'other', message: 'calls go to contacts only' });
    expect(await callStore.start('ab'.repeat(32))).toBeUndefined();
    expect(callStore.error).toEqual({ code: 'other', message: 'calls go to contacts only' });
    expect(callStore.call).toBeNull();
    expect(callStore.busy).toBe(false);
  });

  it('picks up a call taken before the page was there', async () => {
    calls.state.mockResolvedValueOnce({ call: view({ phase: 'connecting', answered_at: 1003 }), policy: 'relay_only', nodes: [], available: true });
    await callStore.load();
    expect(callStore.call?.phase).toBe('connecting');
    expect(callStore.policy).toBe('relay_only');
    expect(callStore.available).toBe(true);
  });

  it('keeps the other own nodes when one is added, without their keys', async () => {
    const own = (reference: string) => ({ reference, id: reference.split('#')[1], class: 'own', has_key: true });
    calls.state.mockResolvedValueOnce({ call: null, policy: 'auto', nodes: [own('1.2.3.4:1#aa'), { ...own('5.6.7.8:2#bb'), class: 'project' }], available: true });
    await callStore.load();
    calls.setNodes.mockResolvedValueOnce({ call: null, policy: 'auto', nodes: [], available: true });
    await callStore.addNode(' 9.9.9.9:3#cc ', ' k ');
    expect(calls.setNodes).toHaveBeenCalledWith([{ reference: '9.9.9.9:3#cc', key: 'k' }, { reference: '1.2.3.4:1#aa' }]);
  });

  it('turns the loudspeaker off toward a headset when there is one', async () => {
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ phase: 'active' }) } });
    callStore.handleEvent({ name: 'call.audio_route', payload: { current: 'speaker', available: ['earpiece', 'speaker', 'bluetooth'] } });
    calls.audioRoute.mockResolvedValueOnce({ current: 'bluetooth', available: ['earpiece', 'speaker', 'bluetooth'] });
    await callStore.toggleSpeaker();
    expect(calls.audioRoute).toHaveBeenCalledWith('set', 'bluetooth');
    expect(callStore.routes?.current).toBe('bluetooth');
  });

  it('turns my camera on and off, and puts the screen in its place and back', async () => {
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ phase: 'active', answered_at: 1005, camera: '/dev/video2' }) } });
    calls.setVideo.mockImplementation(async (input: { kind: string }) => view({
      phase: 'active', answered_at: 1005, camera: '/dev/video2', video_local: input.kind !== 'off', video_screen: input.kind === 'screen',
    }));
    await callStore.toggleCamera();
    expect(calls.setVideo).toHaveBeenLastCalledWith({ kind: 'camera', id: '/dev/video2' });
    expect(callStore.call?.video_local).toBe(true);

    calls.shareScreen.mockResolvedValue(view({ phase: 'active', answered_at: 1005, camera: '/dev/video2', video_local: true, video_screen: true }));
    await callStore.shareScreen('window:7');
    expect(calls.shareScreen).toHaveBeenCalledWith('window:7');
    expect(callStore.call?.video_screen).toBe(true);
    await callStore.stopScreen();
    expect(calls.setVideo, 'the camera was on before the screen').toHaveBeenLastCalledWith({ kind: 'camera', id: '/dev/video2' });

    await callStore.toggleCamera();
    expect(calls.setVideo).toHaveBeenLastCalledWith({ kind: 'off' });
    expect(callStore.call?.video_local).toBe(false);

    await callStore.shareScreen();
    await callStore.stopScreen();
    expect(calls.setVideo, 'the camera was off before the screen').toHaveBeenLastCalledWith({ kind: 'off' });
  });

  it('takes a video call without my camera', async () => {
    callStore.handleEvent({ name: 'call.incoming', payload: { call: view({ media: 'video', video_remote: true }) } });
    calls.accept.mockResolvedValue(view({ media: 'video', phase: 'connecting', video_remote: true, video_local: true }));
    calls.setVideo.mockResolvedValue(view({ media: 'video', phase: 'connecting', video_remote: true, video_local: false }));
    await callStore.acceptWithoutVideo();
    expect(calls.setVideo).toHaveBeenCalledWith({ kind: 'off' });
    expect(callStore.call?.video_local).toBe(false);
    expect(callStore.video, "the peer's video still shows").toBe(true);
  });

  it('shows no video while a video call only rings here', () => {
    callStore.handleEvent({ name: 'call.incoming', payload: { call: view({ media: 'video', video_remote: true }) } });
    expect(callStore.video).toBe(false);
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ media: 'video', phase: 'connecting', video_remote: true }) } });
    expect(callStore.video).toBe(true);
  });
});

/** A promise resolved (or rejected) by hand: an answer that comes when the test says. */
function later<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

const state = (call: CallView | null) => ({ call, policy: 'auto', nodes: [], available: true });

describe('the call store against answers that come late', () => {
  beforeEach(() => {
    callStore.reset();
    vi.clearAllMocks();
  });

  it('keeps a call that rang while the snapshot without it was on its way', async () => {
    const snap = later<ReturnType<typeof state>>();
    calls.state.mockReturnValueOnce(snap.promise);
    const loading = callStore.load();
    callStore.handleEvent({ name: 'call.incoming', payload: { call: view() } });
    snap.resolve(state(null));
    await loading;
    expect(callStore.ringing, 'the ringing card stays').toBe(true);
    expect(callStore.loaded).toBe(true);
  });

  it('does not go back to connecting when active came before the snapshot', async () => {
    const snap = later<ReturnType<typeof state>>();
    calls.state.mockReturnValueOnce(snap.promise);
    const loading = callStore.load();
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ phase: 'active', answered_at: 1003 }) } });
    snap.resolve(state(view({ phase: 'connecting', answered_at: 1003 })));
    await loading;
    expect(callStore.call?.phase).toBe('active');
  });

  it('still drops a call the runtime no longer has when nothing came meanwhile', async () => {
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ phase: 'active' }) } });
    calls.state.mockResolvedValueOnce(state(null));
    await callStore.load();
    expect(callStore.call).toBeNull();
  });

  it('does not bring back a call that ended while the snapshot was on its way, even after its end is no longer shown', async () => {
    vi.useFakeTimers();
    try {
      callStore.handleEvent({ name: 'call.incoming', payload: { call: view() } });
      callStore.handleEvent({ name: 'call.ended', payload: { call: view(), outcome: 'missed', duration_secs: null } });
      vi.advanceTimersByTime(5000);
      expect(callStore.ended).toBeNull();
      calls.state.mockResolvedValueOnce(state(view()));
      await callStore.load();
      expect(callStore.call, 'no ghost that rings on').toBeNull();
      callStore.handleEvent({ name: 'call.state', payload: { call: view() } });
      expect(callStore.call, 'nor a late state of it').toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not take the answer of an accept for a call that ended meanwhile', async () => {
    callStore.handleEvent({ name: 'call.incoming', payload: { call: view() } });
    const answer = later<CallView>();
    calls.accept.mockReturnValueOnce(answer.promise);
    const accepting = callStore.accept();
    callStore.handleEvent({ name: 'call.ended', payload: { call: view(), outcome: 'missed', duration_secs: null } });
    answer.resolve(view({ phase: 'connecting', answered_at: 1003 }));
    await accepting;
    expect(callStore.call).toBeNull();
  });

  it('keeps active when it came before the accept answered connecting', async () => {
    callStore.handleEvent({ name: 'call.incoming', payload: { call: view() } });
    const answer = later<CallView>();
    calls.accept.mockReturnValueOnce(answer.promise);
    const accepting = callStore.accept();
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ phase: 'active', answered_at: 1003 }) } });
    answer.resolve(view({ phase: 'connecting', answered_at: 1003 }));
    await accepting;
    expect(callStore.call?.phase).toBe('active');
  });

  it('asks the runtime again when a decline fails, and lets the ghost go', async () => {
    callStore.handleEvent({ name: 'call.incoming', payload: { call: view() } });
    calls.decline.mockRejectedValueOnce({ code: 'invalid', message: 'no such call is ringing' });
    calls.state.mockResolvedValueOnce(state(null));
    await callStore.decline();
    expect(calls.state).toHaveBeenCalled();
    expect(callStore.call).toBeNull();
    expect(callStore.error, 'a refusal about a call that is gone').toBeNull();
  });

  it('keeps the call and the refusal when the runtime still has the call', async () => {
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ phase: 'active' }) } });
    calls.end.mockRejectedValueOnce({ code: 'other', message: 'boom' });
    calls.state.mockResolvedValueOnce(state(view({ phase: 'active' })));
    await callStore.hangUp();
    expect(callStore.call?.phase).toBe('active');
    expect(callStore.error).toEqual({ code: 'other', message: 'boom' });
  });

  it('hangs up a call that is still being placed, and the start does not bring it back', async () => {
    const answer = later<CallView>();
    calls.start.mockReturnValueOnce(answer.promise);
    const starting = callStore.start('ab'.repeat(32));
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ direction: 'out', phase: 'outgoing' }) } });
    expect(callStore.busy).toBe(true);
    expect(callStore.ending, 'End is not held by the start').toBe(false);
    await callStore.hangUp();
    expect(calls.end).toHaveBeenCalledWith('c1');
    expect(callStore.call).toBeNull();
    answer.resolve(view({ direction: 'out', phase: 'outgoing' }));
    await starting;
    expect(callStore.call, 'the answer of the start comes after the end').toBeNull();
  });

  it('says no refusal when the start fails because the call was hung up meanwhile', async () => {
    const answer = later<CallView>();
    calls.start.mockReturnValueOnce(answer.promise);
    const dialing = callStore.dial('ab'.repeat(32));
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ direction: 'out', phase: 'outgoing' }) } });
    await callStore.hangUp();
    answer.reject({ code: 'invalid', message: 'the call ended' });
    expect(await dialing).toEqual({ view: null, refusal: null });
    expect(callStore.error).toBeNull();
  });

  it('hands a refusal to the button that dialled and keeps none for other screens', async () => {
    calls.start.mockRejectedValueOnce({ code: 'other', message: 'calls go to contacts only' });
    const r = await callStore.dial('ab'.repeat(32));
    expect(r).toEqual({ view: null, refusal: { code: 'other', message: 'calls go to contacts only' } });
    expect(callStore.error).toBeNull();
  });
});

describe('whose end it was, and the settings of this device', () => {
  beforeEach(() => {
    callStore.reset();
    vi.clearAllMocks();
  });

  it('knows my own hang-up as mine, even when the runtime\'s word of it comes first', async () => {
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ direction: 'out', phase: 'outgoing' }) } });
    calls.end.mockImplementationOnce(async () => {
      callStore.handleEvent({ name: 'call.ended', payload: { call: view({ direction: 'out', phase: 'ended' }), outcome: 'missed', duration_secs: null } });
    });
    await callStore.hangUp();
    expect(callStore.ended).toMatchObject({ outcome: 'missed', local: true });
  });

  it('knows my giving up from the notification or a headset as mine, though no command of this page ended it', () => {
    const begun = Math.floor(Date.now() / 1000) - 10;
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ direction: 'out', phase: 'outgoing', started_at: begun }) } });
    callStore.handleEvent({ name: 'call.ended', payload: { call: view({ direction: 'out', phase: 'ended', started_at: begun }), outcome: 'missed', duration_secs: null } });
    expect(callStore.ended).toMatchObject({ outcome: 'missed', local: true });
    expect(endSound(callStore.ended!), 'the falling chime, not the "no"').toBe('end');
  });

  it('knows a ring that ran out as not mine', () => {
    const begun = Math.floor(Date.now() / 1000) - RING_TIMEOUT_SECS;
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ direction: 'out', phase: 'outgoing', started_at: begun }) } });
    callStore.handleEvent({ name: 'call.ended', payload: { call: view({ direction: 'out', phase: 'ended', started_at: begun }), outcome: 'missed', duration_secs: null } });
    expect(callStore.ended).toMatchObject({ outcome: 'missed', local: false });
    expect(endSound(callStore.ended!), 'no answer').toBe('busy');
  });

  it('takes only my unanswered call, "missed" before its ring ran out, for one I gave up', () => {
    const out = view({ direction: 'out', phase: 'ended', started_at: 1000 });
    expect(gaveUp(out, 'missed', 1010)).toBe(true);
    expect(gaveUp(out, 'missed', 1000 + RING_TIMEOUT_SECS), 'the ring ran out').toBe(false);
    for (const o of ['declined', 'busy', 'failed', 'ended', 'answered_elsewhere'] as const) expect(gaveUp(out, o, 1010), o).toBe(false);
    expect(gaveUp({ ...out, answered_at: 1005 }, 'missed', 1010), 'answered').toBe(false);
    expect(gaveUp({ ...out, direction: 'in' }, 'missed', 1010), 'a call to me').toBe(false);
  });

  it('knows a refusal of the peer as not mine', () => {
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ direction: 'out', phase: 'outgoing' }) } });
    callStore.handleEvent({ name: 'call.ended', payload: { call: view({ direction: 'out', phase: 'ended' }), outcome: 'declined', duration_secs: null } });
    expect(callStore.ended).toMatchObject({ outcome: 'declined', local: false });
  });

  it('does not take the next end for mine after a hang-up that failed', async () => {
    callStore.handleEvent({ name: 'call.state', payload: { call: view({ phase: 'active', answered_at: 1005 }) } });
    calls.end.mockRejectedValueOnce({ code: 'other', message: 'boom' });
    calls.state.mockResolvedValueOnce(state(view({ phase: 'active', answered_at: 1005 })));
    await callStore.hangUp();
    callStore.handleEvent({ name: 'call.ended', payload: { call: view({ phase: 'ended' }), outcome: 'ended', duration_secs: 9 } });
    expect(callStore.ended).toMatchObject({ outcome: 'ended', local: false });
  });

  it('offers "calls ring here" only when the runtime says it, and sets it', async () => {
    calls.state.mockResolvedValueOnce(state(null));
    await callStore.load();
    expect(callStore.incoming, 'a runtime without the setting').toBeNull();
    calls.state.mockResolvedValueOnce({ ...state(null), incoming_enabled: true });
    await callStore.load();
    expect(callStore.incoming).toBe(true);
    calls.setIncoming.mockResolvedValueOnce({ ...state(null), incoming_enabled: false });
    await callStore.setIncoming(false);
    expect(calls.setIncoming).toHaveBeenCalledWith(false);
    expect(callStore.incoming).toBe(false);
  });

  it('keeps "ask before calling" on this device', () => {
    const kept = new Map<string, string>();
    vi.stubGlobal('localStorage', { getItem: (k: string) => kept.get(k) ?? null, setItem: (k: string, v: string) => void kept.set(k, v) });
    try {
      expect(callStore.confirm, 'on unless turned off').toBe(true);
      callStore.setConfirm(false);
      expect(callStore.confirm).toBe(false);
      expect(kept.get(CONFIRM_KEY)).toBe('0');
      callStore.setConfirm(true);
      expect(kept.get(CONFIRM_KEY)).toBe('1');
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

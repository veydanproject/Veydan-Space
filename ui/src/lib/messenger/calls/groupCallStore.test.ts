// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The store of group calls follows the runtime's events: the room I am in,
// the banners of the calls on in my groups, how loud each seat is; a room
// left shows how it ended for a moment; an answer older than an event does
// not take the room back; a node without simulcast is asked for no layer;
// a call left (or failed to enter) is entered again under the same id; a
// phone's Hang up on the notification is my leaving.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { GroupCallAnnounced, GroupCallView, GroupParticipant } from '../generated/calls';

const groupCalls = vi.hoisted(() => ({
  state: vi.fn(),
  start: vi.fn(),
  join: vi.fn(),
  leave: vi.fn(async () => {}),
  mute: vi.fn(),
  setVideo: vi.fn(),
  switchCamera: vi.fn(),
  setLayer: vi.fn(async () => {}),
}));
vi.mock('../api', () => ({ messengerApi: { groupCalls } }));

const { groupCallStore } = await import('./groupCallStore.svelte');
const { FOCUS_HOLD_MS } = await import('./group');
const { VIDEO_LOST_MS } = await import('./video');

const me: GroupParticipant = { id: 1, npub: 'ab'.repeat(32), verified: true, speaking: false, audio: true, me: true };
const boris: GroupParticipant = { id: 2, npub: '2b'.repeat(32), verified: true, speaking: false, audio: true, audio_mid: '1', video_mid: '2', me: false };

const A = '1.2.3.4:8443#' + 'cd'.repeat(32);
const B = '5.6.7.8:8443#' + 'ef'.repeat(32);

function view(over: Partial<GroupCallView> = {}): GroupCallView {
  return {
    call_id: 'g1', group_id: 'grp', chat_id: 'group:grp', phase: 'in_room', media: 'audio', muted: false, video_local: false,
    started_by: me.npub!, started_at: 1000, joined_at: 1001, node: A, home: A, participant: 1, epoch: 1,
    participants: [me, boris], kbps_per_participant: 0, max_participants: 0, ...over,
  };
}

function announced(over: Partial<GroupCallAnnounced> = {}): GroupCallAnnounced {
  return { call_id: 'g1', group_id: 'grp', chat_id: 'group:grp', media: 'audio', started_by: boris.npub!, started_at: 1000, participants: [boris.npub!], joined: false, ...over };
}

const state = (call: GroupCallView) => groupCallStore.handleEvent({ name: 'group_call.state', payload: { call } });

beforeEach(() => {
  groupCallStore.reset();
  vi.clearAllMocks();
});

describe('the banner of a group', () => {
  it('shows a call on until it ends', () => {
    groupCallStore.handleEvent({ name: 'group_call.started', payload: { call: announced() } });
    expect(groupCallStore.announcedIn('grp')?.call_id).toBe('g1');
    expect(groupCallStore.announcedIn('other')).toBeNull();
    groupCallStore.handleEvent({ name: 'group_call.started', payload: { call: announced({ participants: [boris.npub!, 'cc'.repeat(32)] }) } });
    expect(groupCallStore.announcedIn('grp')?.participants.length).toBe(2);
    groupCallStore.handleEvent({ name: 'group_call.ended', payload: { call: announced(), outcome: 'ended', duration_secs: 60 } });
    expect(groupCallStore.announcedIn('grp')).toBeNull();
  });

  it('is read from the runtime when the chat opens, unless an event came meanwhile', async () => {
    groupCallStore.handleEvent({ name: 'group_call.started', payload: { call: announced() } });
    groupCalls.state.mockResolvedValueOnce({ call: null, announced: null });
    await groupCallStore.load('grp');
    expect(groupCallStore.announcedIn('grp')).toBeNull();

    let answer: (v: unknown) => void = () => {};
    groupCalls.state.mockReturnValueOnce(new Promise((r) => (answer = r)));
    const loading = groupCallStore.load('grp');
    groupCallStore.handleEvent({ name: 'group_call.started', payload: { call: announced({ call_id: 'g2' }) } });
    answer({ call: null, announced: null });
    await loading;
    expect(groupCallStore.announcedIn('grp')?.call_id).toBe('g2');
  });
});

describe('the room I am in', () => {
  it('follows the events, levels by seat, the last speaker', () => {
    state(view({ phase: 'joining', participants: [me] }));
    expect(groupCallStore.call?.phase).toBe('joining');
    expect(groupCallStore.inGroup('grp')).not.toBeNull();
    state(view({ participants: [me, { ...boris, speaking: true }] }));
    expect(groupCallStore.speaker).toBe(2);
    state(view());
    expect(groupCallStore.speaker, 'kept after they stop').toBe(2);
    groupCallStore.handleEvent({ name: 'group_call.level', payload: { call_id: 'g1', participant: 2, level: 3 } });
    groupCallStore.handleEvent({ name: 'group_call.level', payload: { call_id: 'other', participant: 2, level: 0.1 } });
    expect(groupCallStore.levels[2]).toBe(1);
    // Every seat has a video m-line from the start: video only once its pictures come.
    expect(groupCallStore.video).toBe(false);
    groupCallStore.seatPicture('2');
    expect(groupCallStore.video).toBe(true);
  });

  it('is entered again after I left it: the call keeps its id from one join to the next', async () => {
    state(view());
    await groupCallStore.leave();
    state(view({ phase: 'left', participants: [] }));
    expect(groupCallStore.call).toBeNull();
    groupCalls.join.mockImplementationOnce(async () => {
      state(view({ phase: 'joining', participants: [me] }));
      return view({ phase: 'joining', participants: [me] });
    });
    const { view: got, refusal } = await groupCallStore.join('grp');
    expect(refusal).toBeNull();
    expect(got?.call_id).toBe('g1');
    expect(groupCallStore.call?.phase).toBe('joining');
    expect(groupCallStore.over, 'how the last time ended is gone').toBeNull();
    state(view());
    expect(groupCallStore.call?.phase).toBe('in_room');
    expect(groupCallStore.inGroup('grp')?.call_id).toBe('g1');
  });

  it('is entered after a first join that failed; the failure is the refusal, no room screen', async () => {
    groupCalls.join.mockImplementationOnce(async () => {
      state(view({ phase: 'joining', participants: [me] }));
      state(view({ phase: 'left', participants: [] }));
      throw { code: 'transport', message: 'the way to the node is lost' };
    });
    const first = await groupCallStore.join('grp');
    expect(first.refusal).toMatchObject({ message: /way to the node/ });
    expect(groupCallStore.call).toBeNull();
    expect(groupCallStore.over, 'the banner says it').toBeNull();
    groupCalls.join.mockImplementationOnce(async () => {
      state(view({ phase: 'joining', participants: [me] }));
      return view({ phase: 'joining', participants: [me] });
    });
    await groupCallStore.join('grp');
    state(view());
    expect(groupCallStore.call?.phase).toBe('in_room');
  });

  it('hands back the refusal of a call already over, after the banner went', async () => {
    groupCallStore.handleEvent({ name: 'group_call.started', payload: { call: announced() } });
    groupCalls.join.mockImplementationOnce(async () => {
      groupCallStore.handleEvent({ name: 'group_call.ended', payload: { call: announced(), outcome: 'failed', duration_secs: null } });
      throw { code: 'transport', message: 'POST /v1/rooms/ab/join: 404 room_not_found' };
    });
    const { refusal } = await groupCallStore.join('grp');
    expect(groupCallStore.announcedIn('grp')).toBeNull();
    expect(refusal).toMatchObject({ message: /room_not_found/ });
  });

  it('is not taken back by an answer older than an event', async () => {
    let answer: (v: GroupCallView) => void = () => {};
    groupCalls.join.mockReturnValueOnce(new Promise((r) => (answer = r)));
    const joining = groupCallStore.join('grp');
    state(view());
    answer(view({ phase: 'joining' }));
    const { view: got, refusal } = await joining;
    expect(refusal).toBeNull();
    expect(got?.phase).toBe('joining');
    expect(groupCallStore.call?.phase).toBe('in_room');
  });

  it('hands a refusal back to the button that asked', async () => {
    groupCalls.start.mockRejectedValueOnce({ code: 'invalid', message: 'a call is on in this group already: join it' });
    const { view: got, refusal } = await groupCallStore.start('grp', 'video');
    expect(got).toBeNull();
    expect(refusal).toMatchObject({ message: /already/ });
    expect(groupCallStore.error, 'not kept for another screen').toBeNull();
  });

  it('shows how it ended for a moment: left by me, gone, never come about', async () => {
    state(view());
    await groupCallStore.leave();
    expect(groupCalls.leave).toHaveBeenCalled();
    expect(groupCallStore.call).toBeNull();
    expect(groupCallStore.over?.how).toBe('left');
    // The runtime's own word of it changes nothing; a late state is no new room.
    state(view({ phase: 'left', participants: [] }));
    state(view());
    expect(groupCallStore.over?.how).toBe('left');
    expect(groupCallStore.call).toBeNull();

    groupCallStore.reset();
    state(view({ call_id: 'g2' }));
    state(view({ call_id: 'g2', phase: 'left', participants: [] }));
    expect(groupCallStore.over?.how).toBe('ended');

    groupCallStore.reset();
    state(view({ call_id: 'g3', phase: 'starting', participants: [] }));
    state(view({ call_id: 'g3', phase: 'left', participants: [] }));
    expect(groupCallStore.over?.how).toBe('failed');
  });

  it('asks a layer once per change, and no more of a node without simulcast', async () => {
    state(view());
    await groupCallStore.setLayer(2, 'q');
    await groupCallStore.setLayer(2, 'q');
    await groupCallStore.setLayer(2, 'h');
    expect(groupCalls.setLayer.mock.calls).toEqual([[2, 'q'], [2, 'h']]);
    groupCalls.setLayer.mockRejectedValueOnce({ code: 'invalid', message: 'the node has no simulcast' });
    await groupCallStore.setLayer(2, 'f');
    await groupCallStore.setLayer(2, 'q');
    expect(groupCalls.setLayer).toHaveBeenCalledTimes(3);
  });

  it('turns my camera off and on, my screen in its place and back', async () => {
    state(view());
    groupCalls.setVideo.mockImplementation(async (input: { kind: string }) => view({ video_local: input.kind !== 'off' }));
    await groupCallStore.toggleCamera();
    expect(groupCalls.setVideo).toHaveBeenLastCalledWith({ kind: 'camera' });
    expect(groupCallStore.call?.video_local).toBe(true);
    await groupCallStore.shareScreen('screen:0');
    expect(groupCallStore.screen).toBe(true);
    await groupCallStore.stopScreen();
    expect(groupCalls.setVideo).toHaveBeenLastCalledWith({ kind: 'camera' });
    expect(groupCallStore.screen).toBe(false);
    await groupCallStore.toggleCamera();
    expect(groupCalls.setVideo).toHaveBeenLastCalledWith({ kind: 'off' });
  });
});

describe('the move of a room', () => {
  it('is told from a way merely lost, keeps the screen, asks the new seats anew', async () => {
    state(view());
    groupCallStore.shown = false;
    await groupCallStore.setLayer(2, 'f');
    expect(groupCalls.setLayer).toHaveBeenCalledTimes(1);
    // The way lost: the same home, no move (yet).
    state(view({ phase: 'reconnecting' }));
    expect(groupCallStore.moving).toBe(false);
    // Through my node B into the same room: no move either.
    state(view({ phase: 'joining', node: B, home: A }));
    expect(groupCallStore.moving).toBe(false);
    state(view({ phase: 'in_room', node: B, home: A }));
    // The room's node went: a room on B under the same call.
    state(view({ phase: 'reconnecting', node: B, home: A }));
    expect(groupCallStore.moving).toBe(false);
    state(view({ phase: 'joining', node: B, home: B, epoch: 2, participants: [me] }));
    expect(groupCallStore.moving).toBe(true);
    expect(groupCallStore.call?.call_id).toBe('g1');
    expect(groupCallStore.over).toBeNull();
    expect(groupCallStore.shown).toBe(false);
    // Seat 2 of the new room is somebody else: asked again.
    await groupCallStore.setLayer(2, 'f');
    expect(groupCalls.setLayer).toHaveBeenCalledTimes(2);
    state(view({ phase: 'in_room', node: B, home: B, epoch: 2 }));
    expect(groupCallStore.moving).toBe(false);
  });

  it('says the same in every place that shows the room: the move, not a join', () => {
    const tr = (key: string) => key;
    state(view());
    expect(groupCallStore.statusText(tr)).not.toMatch(/^msg_/);
    state(view({ phase: 'reconnecting' }));
    expect(groupCallStore.statusText(tr)).toBe('msg_call_phase_reconnecting');
    // After the move the runtime says `joining` under the new home.
    state(view({ phase: 'joining', node: B, home: B, participants: [] }));
    expect(groupCallStore.statusText(tr)).toBe('msg_gcall_phase_moving');
    state(view({ phase: 'joining', node: B, home: B, participants: [me] }));
    expect(groupCallStore.statusText(tr)).toBe('msg_gcall_phase_moving');
    state(view({ phase: 'in_room', node: B, home: B }));
    expect(groupCallStore.statusText(tr)).not.toMatch(/^msg_/);
    groupCallStore.reset();
    expect(groupCallStore.statusText(tr)).toBe('');
  });

  it('is told by a new room on the same node too (a double move): the seats asked anew', async () => {
    state(view({ node: B, home: B }));
    await groupCallStore.setLayer(2, 'f');
    groupCallStore.seatPicture('2');
    expect(groupCallStore.showing[2]).toBe(true);
    // The winner's room is on B as well: the runtime leaves my room for it,
    // `joining` with no seat at all.
    state(view({ phase: 'joining', node: B, home: B, participants: [] }));
    expect(groupCallStore.moving).toBe(true);
    expect(groupCallStore.showing).toEqual({});
    state(view({ phase: 'joining', node: B, home: B, participant: 3, participants: [{ ...me, id: 3 }] }));
    expect(groupCallStore.moving).toBe(true);
    // Seat 2 of the winner's room is somebody else: asked again.
    await groupCallStore.setLayer(2, 'f');
    expect(groupCalls.setLayer.mock.calls).toEqual([[2, 'f'], [2, 'f']]);
    state(view({ phase: 'in_room', node: B, home: B, participant: 3, participants: [{ ...me, id: 3 }, boris] }));
    expect(groupCallStore.moving).toBe(false);
    // A way merely lost afterwards is no move, and keeps what was asked.
    await groupCallStore.setLayer(2, 'q');
    state(view({ phase: 'reconnecting', node: B, home: B, participant: 3, participants: [{ ...me, id: 3 }, boris] }));
    state(view({ phase: 'joining', node: B, home: B, participant: 3, participants: [{ ...me, id: 3 }] }));
    expect(groupCallStore.moving).toBe(false);
    await groupCallStore.setLayer(2, 'q');
    expect(groupCalls.setLayer).toHaveBeenCalledTimes(3);
  });

  it('is not seen in a room I never talked in, nor without a home (an older runtime)', () => {
    state(view({ phase: 'joining', node: B, home: A, participants: [me] }));
    state(view({ phase: 'joining', node: B, home: B, participants: [me] }));
    expect(groupCallStore.moving).toBe(false);
    state(view({ phase: 'in_room', node: A, home: '' }));
    state(view({ phase: 'reconnecting', node: A, home: '' }));
    expect(groupCallStore.moving).toBe(false);
  });
});

describe('a failure the room tells of', () => {
  it('shows for a while, then goes', () => {
    vi.useFakeTimers();
    try {
      state(view());
      groupCallStore.handleEvent({ name: 'error', payload: { scope: 'group_calls', error: { code: 'transport', message: 'no camera on this machine' } } });
      groupCallStore.handleEvent({ name: 'error', payload: { scope: 'calls', error: 'not mine' } });
      expect(groupCallStore.error).toMatchObject({ message: /camera/ });
      vi.advanceTimersByTime(7000);
      expect(groupCallStore.error).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps which seats show pictures, from their tiles', () => {
    state(view());
    groupCallStore.seatPicture('2');
    expect(groupCallStore.showing[2]).toBe(true);
    state(view({ call_id: 'g9' }));
    expect(groupCallStore.showing[2], 'a new room starts with none').toBeUndefined();
  });
});

describe('the large place of the room', () => {
  const alice: GroupParticipant = { id: 3, npub: '3c'.repeat(32), verified: true, speaking: false, audio: true, audio_mid: '3', video_mid: '4', me: false };
  const vera: GroupParticipant = { id: 4, npub: '4d'.repeat(32), verified: true, speaking: false, audio: true, audio_mid: '5', video_mid: '6', me: false };
  const talk = (...ids: number[]) => state(view({ participants: [me, boris, alice, vera].map((p) => ({ ...p, speaking: ids.includes(p.id) })) }));

  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('stays with a camera that is on while its frames pause: off only after VIDEO_LOST_MS without a picture', () => {
    talk();
    groupCallStore.seatPicture('2');
    talk(2);
    expect(groupCallStore.voice).toBe(2);
    // The frames of Boris pause (a layer changes, the way's bandwidth): his
    // tile keeps its last picture, the room's words go on, nothing moves.
    vi.advanceTimersByTime(VIDEO_LOST_MS - 1500);
    talk();
    talk(2);
    expect(groupCallStore.voice).toBe(2);
    expect(groupCallStore.video).toBe(true);
    // A picture again: his clock starts anew.
    groupCallStore.seatPicture('2');
    vi.advanceTimersByTime(VIDEO_LOST_MS - 1500);
    expect(groupCallStore.showing[2]).toBe(true);
    // Missing for long: his camera is taken as off, the large place let go.
    vi.advanceTimersByTime(2000);
    expect(groupCallStore.showing[2]).toBe(false);
    expect(groupCallStore.voice).toBeNull();
    expect(groupCallStore.video).toBe(false);
    // His next picture: on again.
    groupCallStore.seatPicture('2');
    expect(groupCallStore.showing[2]).toBe(true);
  });

  it('counts a camera off from its last picture, not from its tile: two who talk in turns do not keep it on', () => {
    // Boris and Alice, both cameras on; Alice large.
    talk();
    groupCallStore.seatPicture('2');
    groupCallStore.seatPicture('4');
    talk(3);
    expect(groupCallStore.voice).toBe(3);
    // Alice turns her camera off at 0 (her m-line stays, her pictures
    // stop); Boris's keep coming. Every change of the large place makes
    // the tiles of both anew: the clock of a seat is not theirs.
    for (let s = 1; s <= 12; s++) {
      vi.advanceTimersByTime(1000);
      groupCallStore.seatPicture('2');
      if (s === 4) talk(2);
      if (s === 8) talk(3);
      if (s === 4) expect(groupCallStore.voice).toBe(2);
      if (s === 8) expect(groupCallStore.voice, 'still on, short of VIDEO_LOST_MS').toBe(3);
    }
    // Ten seconds after her last picture her camera is off, whoever spoke meanwhile.
    expect(groupCallStore.showing[3]).toBe(false);
    talk(2);
    talk(3);
    vi.advanceTimersByTime(FOCUS_HOLD_MS);
    expect(groupCallStore.voice, 'Alice speaks without a picture: Boris stays large').toBe(2);
  });

  it('takes the word of a tile by the m-line of its subscription, not by the seat it shows now', () => {
    talk();
    groupCallStore.seatPicture('2');
    groupCallStore.seatPicture('4');
    talk(3);
    expect(groupCallStore.voice).toBe(3);
    // Alice's stream ends: her camera is off at once, the large place goes
    // to Boris; the end said by the m-line of Alice touches no other seat.
    groupCallStore.seatEnded('4');
    expect(groupCallStore.showing[3]).toBe(false);
    talk(2);
    expect(groupCallStore.voice).toBe(2);
    groupCallStore.seatEnded('4');
    expect(groupCallStore.showing[2]).toBe(true);
    expect(groupCallStore.voice).toBe(2);
    // An m-line of no seat (a room that moved on), or mine, says nothing.
    groupCallStore.seatPicture('99');
    groupCallStore.seatEnded('99');
    expect(groupCallStore.showing).toEqual({ 2: true, 3: false });
  });

  it('holds the speaker a while: two who talk in turns do not throw the screen to and fro', () => {
    talk();
    groupCallStore.seatPicture('2');
    groupCallStore.seatPicture('4');
    talk(2);
    expect(groupCallStore.voice).toBe(2);
    vi.advanceTimersByTime(1000);
    talk(3);
    expect(groupCallStore.voice, 'held').toBe(2);
    talk(2);
    talk(3);
    expect(groupCallStore.voice).toBe(2);
    // The hold over, with Alice still speaking: the place goes to her by itself.
    vi.advanceTimersByTime(2000);
    expect(groupCallStore.voice).toBe(3);
    // A voice without a picture speaks: the last picture stays large.
    talk(4);
    vi.advanceTimersByTime(5000);
    expect(groupCallStore.voice).toBe(3);
  });

  it('lets a seat go at once when it leaves, or when the m-line of its video goes', () => {
    talk();
    groupCallStore.seatPicture('2');
    groupCallStore.seatPicture('4');
    talk(2);
    talk(3);
    expect(groupCallStore.voice).toBe(2);
    // Boris leaves: no hold for a seat that is not there.
    state(view({ participants: [me, { ...alice, speaking: true }, vera] }));
    expect(groupCallStore.voice).toBe(3);
    expect(groupCallStore.showing[2], 'his seat is let go').toBeUndefined();
    // Alice's video has no m-line any more: her camera is off, the room changes at once.
    state(view({ participants: [me, { ...alice, video_mid: undefined, speaking: true }, vera] }));
    expect(groupCallStore.showing[3]).toBeUndefined();
    expect(groupCallStore.voice).toBeNull();
    expect(groupCallStore.video).toBe(false);
  });

  it('is let go with the room, its look again too', () => {
    talk();
    groupCallStore.seatPicture('2');
    groupCallStore.seatPicture('4');
    talk(2);
    talk(3);
    groupCallStore.reset();
    vi.advanceTimersByTime(5000);
    expect(groupCallStore.voice).toBeNull();
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('the phone', () => {
  it('learns where the sound can go, and moves it to the loudspeaker and back', async () => {
    const calls = { audioRoute: vi.fn() };
    (await import('../api')).messengerApi.calls = calls as never;
    state(view());
    calls.audioRoute.mockResolvedValueOnce({ current: 'earpiece', available: ['earpiece', 'speaker'] });
    await groupCallStore.loadRoutes();
    expect(groupCallStore.routes?.available).toContain('speaker');
    calls.audioRoute.mockResolvedValueOnce({ current: 'speaker', available: ['earpiece', 'speaker'] });
    await groupCallStore.toggleSpeaker();
    expect(calls.audioRoute).toHaveBeenLastCalledWith('set', 'speaker');
    calls.audioRoute.mockResolvedValueOnce({ current: 'earpiece', available: ['earpiece', 'speaker'] });
    await groupCallStore.toggleSpeaker();
    expect(calls.audioRoute).toHaveBeenLastCalledWith('set', 'earpiece');
    // A headset came: the phone's own word.
    groupCallStore.handleEvent({ name: 'call.audio_route', payload: { current: 'wired', available: ['wired', 'speaker'] } });
    expect(groupCallStore.routes?.current).toBe('wired');
    // A computer refuses: nothing is shown.
    calls.audioRoute.mockRejectedValueOnce(new Error('the route of the sound is the phone\'s'));
    await groupCallStore.loadRoutes();
    expect(groupCallStore.routes).toBeNull();
    state(view({ call_id: 'g9' }));
    expect(groupCallStore.routes, 'a new room asks anew').toBeNull();
  });

  it('shows the camera the phone refused for the room, for a while', () => {
    vi.useFakeTimers();
    try {
      state(view());
      groupCallStore.handleEvent({ name: 'call.camera_failed', payload: { callId: 'other', error: 'not mine', denied: false } });
      expect(groupCallStore.error).toBeNull();
      groupCallStore.handleEvent({ name: 'call.camera_failed', payload: { callId: 'g1', error: 'the camera did not open', denied: false } });
      expect(groupCallStore.error).toMatch(/camera/);
      vi.advanceTimersByTime(7000);
      expect(groupCallStore.error).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps the room when my camera failed on a phone: the shell turned my video off, the call goes on', () => {
    vi.useFakeTimers();
    try {
      state(view({ media: 'video', video_local: true }));
      groupCallStore.handleEvent({ name: 'call.camera_failed', payload: { callId: 'g1', error: 'the camera is not allowed', denied: true } });
      expect(groupCallStore.error).toMatch(/camera/);
      state(view({ media: 'video', video_local: false }));
      expect(groupCallStore.call?.video_local).toBe(false);
      vi.advanceTimersByTime(7000);
      expect(groupCallStore.error).toBeNull();
      state(view({ media: 'video', phase: 'left', participants: [] }));
      expect(groupCallStore.over?.how, 'not failed').toBe('ended');
    } finally {
      vi.useRealTimers();
    }
  });

  it('flips the camera through the runtime and takes its word of the room', async () => {
    state(view({ media: 'video', video_local: true, camera: 'front' }));
    groupCalls.switchCamera.mockResolvedValueOnce(view({ media: 'video', video_local: true, camera: 'back' }));
    await groupCallStore.switchCamera();
    expect(groupCalls.switchCamera).toHaveBeenCalledTimes(1);
    expect(groupCallStore.call?.camera).toBe('back');
    groupCallStore.reset();
    await groupCallStore.switchCamera();
    expect(groupCalls.switchCamera, 'nothing without a room').toHaveBeenCalledTimes(1);
  });

  it('tells my Hang up on the notification from the room going', () => {
    state(view());
    groupCallStore.handleEvent({ name: 'call.debug', payload: { action: 'hangup', callId: 'other' } });
    groupCallStore.handleEvent({ name: 'call.debug', payload: { action: 'hangup', callId: 'g1' } });
    groupCallStore.handleEvent({ name: 'error', payload: { scope: 'group_calls', error: 'a way restored' } });
    state(view({ phase: 'left', participants: [] }));
    expect(groupCallStore.over?.how).toBe('left');
    // The end of the call for everybody after that changes nothing: I left it, as with Leave on the screen.
    groupCallStore.handleEvent({ name: 'group_call.ended', payload: { call: announced(), outcome: 'ended', duration_secs: 60 } });
    expect(groupCallStore.over?.how).toBe('left');

    groupCallStore.reset();
    state(view({ call_id: 'g2' }));
    groupCallStore.handleEvent({ name: 'call.debug', payload: { action: 'answer', callId: 'g2' } });
    state(view({ call_id: 'g2', phase: 'left', participants: [] }));
    expect(groupCallStore.over?.how, 'not my press').toBe('ended');
  });
});

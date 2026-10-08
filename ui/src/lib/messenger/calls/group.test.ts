// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { GroupCallView, GroupParticipant } from '../generated/calls';
import {
  MY_VIDEO_MID, bannerKey, focusOf, gridShape, groupElapsed, groupStatusText, layerFor, mirrorsMine, nodeHost, overKey, roomPath, roomPathText, seatMuted,
  seatSendsVideo, seatsInOrder,
} from './group';
import { callErrorText } from './words';

const tr = (key: string) => key;

function seat(id: number, over: Partial<GroupParticipant> = {}): GroupParticipant {
  return { id, npub: `${id}`.repeat(64).slice(0, 64), verified: true, speaking: false, audio: true, me: false, ...over };
}

function room(over: Partial<GroupCallView> = {}): GroupCallView {
  return {
    call_id: 'g1', group_id: 'g', chat_id: 'group:g', phase: 'in_room', media: 'audio', muted: false, video_local: false,
    started_by: 'ab', started_at: 1000, joined_at: 1010, node: '108.61.171.68:8443#' + 'ab'.repeat(32),
    home: '108.61.171.68:8443#' + 'ab'.repeat(32), participant: 1, epoch: 1,
    participants: [], kbps_per_participant: 0, max_participants: 0, ...over,
  };
}

describe('the seats of a room', () => {
  it('go the others first by seat, the unconfirmed after them, mine last', () => {
    const order = seatsInOrder([seat(1, { me: true }), seat(4), seat(2, { verified: false, npub: undefined }), seat(3)]);
    expect(order.map((p) => p.id)).toEqual([3, 4, 2, 1]);
  });

  it('show large the picked seat, else the video of who speaks or spoke last, never mine', () => {
    const list = [seat(1, { me: true, speaking: true, video_mid: 'v1' }), seat(2, { video_mid: 'v2' }), seat(3, { speaking: true }), seat(4, { video_mid: 'v4' })];
    expect(focusOf(list, null, null)).toBeNull();
    // A voice without a picture gains nothing from the large place.
    expect(focusOf(list, null, 3)).toBeNull();
    expect(focusOf(list, null, 2)).toBe(2);
    const talking = list.map((p) => (p.id === 4 ? { ...p, speaking: true } : p));
    expect(focusOf(talking, null, 2)).toBe(4);
    expect(focusOf(talking, 3, 2)).toBe(3);
    // A picked seat that left lets go.
    expect(focusOf(talking, 9, null)).toBe(4);
    // An unconfirmed seat is not shown large by itself.
    expect(focusOf([seat(5, { verified: false, speaking: true, video_mid: 'v5' })], null, 5)).toBeNull();
    // A track that sends no pictures is no video to show large.
    expect(focusOf(talking, null, 2, (id) => id !== 4)).toBe(2);
    expect(focusOf(talking, null, null, () => false)).toBeNull();
  });

  it('are cut into the grid that makes the tiles largest', () => {
    expect(gridShape(1, 800, 500)).toEqual({ cols: 1, rows: 1 });
    expect(gridShape(2, 800, 500)).toEqual({ cols: 2, rows: 1 });
    expect(gridShape(2, 400, 800, 3 / 4)).toEqual({ cols: 1, rows: 2 });
    expect(gridShape(4, 800, 500)).toEqual({ cols: 2, rows: 2 });
    expect(gridShape(5, 1200, 500)).toEqual({ cols: 3, rows: 2 });
    expect(gridShape(3, 0, 0)).toEqual({ cols: 1, rows: 3 });
  });
});

describe('the layer a tile asks for', () => {
  it('is the smallest that is not smaller than the tile', () => {
    expect(layerFor(160, 90)).toBe('q');
    expect(layerFor(320, 180, 1)).toBe('q');
    expect(layerFor(320, 180, 2)).toBe('h');
    expect(layerFor(600, 340)).toBe('h');
    expect(layerFor(1000, 560)).toBe('f');
    // A tall tile is filled by the picture's height.
    expect(layerFor(200, 400)).toBe('h');
  });
});

describe('the words of a room', () => {
  it('say the phase, or the clock from my entry', () => {
    expect(groupStatusText(room({ phase: 'starting' }), null, 2000, tr)).toBe('msg_gcall_phase_starting');
    expect(groupStatusText(room({ phase: 'joining' }), null, 2000, tr)).toBe('msg_gcall_phase_joining');
    expect(groupStatusText(room({ phase: 'reconnecting' }), null, 2000, tr)).toBe('msg_call_phase_reconnecting');
    expect(groupStatusText(room(), null, 1075, tr)).toBe('1:05');
    expect(groupElapsed({ started_at: 1000, joined_at: undefined }, 1030)).toBe(30);
    expect(groupStatusText(null, 'left', 0, tr)).toBe('msg_gcall_over_left');
    expect(overKey('failed')).toBe('msg_call_ended_failed');
    expect(overKey('ended')).toBe('msg_gcall_over_ended');
    expect(groupStatusText(null, null, 0, tr)).toBe('');
  });

  it('say the move while the room is on its way to another node, the clock once it is there', () => {
    expect(groupStatusText(room({ phase: 'reconnecting' }), null, 2000, tr, true)).toBe('msg_gcall_phase_moving');
    expect(groupStatusText(room({ phase: 'joining' }), null, 2000, tr, true)).toBe('msg_gcall_phase_moving');
    expect(groupStatusText(room(), null, 1075, tr, true)).toBe('1:05');
  });

  it('name the way to the room: one node, or mine and the room\'s', () => {
    const B = '149.28.37.154:8443#' + 'cd'.repeat(32);
    const words = (key: string, p?: Record<string, string>) => `${key}(${Object.values(p ?? {}).join(',')})`;
    expect(roomPath(room())).toEqual({ via: null, home: '108.61.171.68' });
    expect(roomPathText(room(), words)).toBe('msg_gcall_via_node(108.61.171.68)');
    expect(roomPath(room({ node: B }))).toEqual({ via: '149.28.37.154', home: '108.61.171.68' });
    expect(roomPathText(room({ node: B }), words)).toBe('msg_gcall_via_cascade(149.28.37.154,108.61.171.68)');
    // A runtime without the cascade says no home: the node is the room's.
    expect(roomPathText(room({ node: B, home: '' }), words)).toBe('msg_gcall_via_node(149.28.37.154)');
    expect(roomPath(room({ node: '', home: '' }))).toBeNull();
  });

  it('name the node by its address alone', () => {
    expect(nodeHost('108.61.171.68:8443#' + 'ab'.repeat(32))).toBe('108.61.171.68');
    expect(nodeHost('[2001:db8::1]:8443#' + 'ab'.repeat(32))).toBe('2001:db8::1');
    expect(nodeHost('call.example.org:443#ff')).toBe('call.example.org');
  });
});

describe('what a seat shows', () => {
  const pictures = (ids: number[]) => (seat: number) => ids.includes(seat);

  it('a camera only while its pictures come: every seat of a room has a video m-line from the start', () => {
    const b = seat(2, { video_mid: '2' });
    // An audio call, or the camera off: the m-line is there, no frames come.
    expect(seatSendsVideo(b, room(), pictures([]))).toBe(false);
    expect(seatSendsVideo(b, room(), pictures([2]))).toBe(true);
    expect(seatSendsVideo(seat(3), room(), pictures([3])), 'no track yet').toBe(false);
    expect(seatSendsVideo(seat(4, { verified: false, video_mid: '4' }), room(), pictures([4])), 'unconfirmed').toBe(false);
    // Mine: my own state.
    expect(seatSendsVideo(seat(1, { me: true }), room({ video_local: true }), pictures([]))).toBe(true);
    expect(seatSendsVideo(seat(1, { me: true, video_mid: '9' }), room(), pictures([1]))).toBe(false);
  });

  it('a crossed microphone for mine when off; never for another from its m-line of sound', () => {
    expect(seatMuted(seat(1, { me: true }), room({ muted: true }))).toBe(true);
    expect(seatMuted(seat(1, { me: true }), room())).toBe(false);
    // `audio` false is a track not come yet, not a microphone off.
    expect(seatMuted(seat(2, { audio: false }), room())).toBe(false);
    expect(seatMuted(seat(2), room({ muted: true })), 'my mute is not theirs').toBe(false);
    // Once the room says another's microphone is off.
    expect(seatMuted({ ...seat(2), muted: true } as GroupParticipant, room())).toBe(true);
    expect(seatMuted({ ...seat(2, { verified: false }), muted: true } as GroupParticipant, room())).toBe(false);
  });
});

describe('the strip of a group', () => {
  it('says which call is on, the same whatever the clock', () => {
    expect(bannerKey(true, 'video')).toBe('msg_gcall_banner_mine');
    expect(bannerKey(false, 'video')).toBe('msg_gcall_banner_video');
    expect(bannerKey(false, 'audio')).toBe('msg_gcall_banner');
    expect(bannerKey(false, undefined)).toBe('msg_gcall_banner');
  });
});

describe('my own tile', () => {
  it('asks the runtime for my frames by a mid no seat of the node can have', () => {
    // An SDP mid is a token: `@` is not among its characters.
    expect(MY_VIDEO_MID).toMatch(/[^A-Za-z0-9!#$%&'*+\-.^_`|~]/);
  });

  it('is mirrored as a face is, not as a screen or the world of a back camera', () => {
    expect(mirrorsMine(room(), false)).toBe(true);
    expect(mirrorsMine(room({ camera: 'front' }), false)).toBe(true);
    expect(mirrorsMine(room({ camera: '/dev/video0' }), false)).toBe(true);
    expect(mirrorsMine(room({ camera: 'back' }), false)).toBe(false);
    expect(mirrorsMine(room(), true)).toBe(false);
  });
});

describe('the words of a camera that failed', () => {
  it('tell a camera that would not open from one that is not there', () => {
    expect(callErrorText({ code: 'other', message: 'camera busy' }, tr)).toBe('msg_call_err_camera');
    expect(callErrorText('the camera is not allowed', tr)).toBe('msg_call_err_camera');
    expect(callErrorText('no camera', tr)).toBe('msg_call_err_no_camera');
  });
});

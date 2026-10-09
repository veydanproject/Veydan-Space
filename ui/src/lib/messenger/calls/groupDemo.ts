// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Group calls of the browser preview (`pnpm dev` without Tauri), played so
// that every screen of a group call can be looked at without a backend.
// Never used inside the app. calls/demo.ts mounts it beside the calls of
// two and lends it its channels of frames.
//
// A call started in the demo fills by itself: a member comes in a moment
// later (nameless until their word of identity is "checked"), then another
// with video; the others speak in turns. One is already on when asked:
//   localStorage['messenger.demo.gcall'] = 'announced' | 'announced-video' | 'in'
// plays it a moment after the page loads (in the first group I am in:
// Boris and Alice are in it; `in`: I am in it too, with video), and
// `window.veydanDemoGroupCall(kind)` plays at once: `announce`,
// `announce-video`, `in`, `come` (a member comes in), `go` (one leaves),
// `video` (Boris's camera on or off: as in the runtime, the m-line of his
// video stays and only its frames stop), `video-gone` (his camera off with
// the m-line of his video gone), `pause` (the frames of the first seat with
// video held back for 15 seconds, its camera on), `blink` (the same for 2
// seconds, as a change of layer or of the way's bandwidth does),
// `reconnect`, `cascade` (my way to the
// room through my nearest node, or straight again), `move` (the room's node
// goes: the room moves to another under the same call, its seats anew),
// `end` (everybody leaves).
//
// The video of a seat is the test picture of calls/video.ts, at the size
// of the layer its tile asked for (`messenger_group_call_set_layer`):
// 320×180, 640×360 or 1280×720.

import type { MessengerMessage } from '../api';
import type { CallMedia, GroupCallAnnounced, GroupCallView, GroupParticipant, VideoInput } from '../generated/calls';
import type { DemoCallHost, DemoChannel } from './demo';
import { MY_VIDEO_MID } from './group';
import { testPattern } from './video';

export interface DemoFrames {
  subscribe: (channel: DemoChannel, make: (seq: number) => ArrayBuffer | null, fps?: number) => number;
  /** The last message to every subscription of a group call. */
  endAll: () => void;
  endNow: (channel: DemoChannel) => number;
  /** A call of two is under way in the demo. */
  dmBusy: () => boolean;
}

const NODE = '108.61.171.68:8443#fda09da75199c4e04601a9df309710fab15cd7b2b806ca3b2bbab202582a6dca';
const NODE_2 = '149.28.37.154:8443#68367a61a90c3efafa3a0d4aec481be7c8c21f266864871ecbe9e8ff139bfb03';
const LAYERS: Record<string, [number, number]> = { q: [320, 180], h: [640, 360], f: [1280, 720] };
const FPS = 15;

interface Seat {
  id: number;
  pk: string;
  verified: boolean;
  video: boolean;
  /** The m-line of the seat's video is there: from the start, kept while the camera is off (as the node has it). */
  mid: boolean;
  /** The seat's frames are held back until then (`Date.now()`), its camera on. */
  pausedUntil?: number;
  speaking: boolean;
  /** The layer the page asked for. */
  layer: string;
}

interface Room {
  call_id: string;
  group_id: string;
  media: CallMedia;
  started_by: string;
  started_at: number;
  /** The others in the room. */
  seats: Seat[];
  /** I am in the room, on `mine`. */
  joined: boolean;
  mine: number;
  phase: GroupCallView['phase'];
  joined_at?: number;
  muted: boolean;
  video_local: boolean;
  /** The camera in use (or for the next time): `front`, `back`, as a phone names them. */
  camera?: string;
  /** How many people the room saw, me included. */
  seen: Set<string>;
  epoch: number;
  /** The node I am on; the node of the room (`home`): another one when I sit in it through mine. */
  node: string;
  home: string;
}

const nowSecs = () => Math.floor(Date.now() / 1000);
const hexId = () => Array.from({ length: 32 }, () => Math.floor(Math.random() * 16).toString(16)).join('');
const refuse = (message: string) => ({ code: 'invalid', message });

export function demoGroupCallMocks(host: DemoCallHost, frames: DemoFrames): {
  mocks: Record<string, (args?: Record<string, unknown>) => unknown>;
  active: () => boolean;
} {
  let room: Room | null = null;
  const timers: ReturnType<typeof setTimeout>[] = [];
  let pulse: ReturnType<typeof setInterval> | null = null;
  let asked = false;

  const later = (ms: number, fn: () => void) => timers.push(setTimeout(fn, ms));
  const chatOf = (groupId: string) => `group:${groupId}`;
  const stopAll = () => {
    while (timers.length) clearTimeout(timers.pop());
    if (pulse) clearInterval(pulse);
    pulse = null;
  };
  const nextSeat = (r: Room) => Math.max(r.joined ? r.mine : 0, ...r.seats.map((s) => s.id)) + 1;

  function participants(r: Room): GroupParticipant[] {
    const me: GroupParticipant = { id: r.mine, npub: host.me(), verified: true, speaking: false, audio: true, me: true };
    return [
      me,
      ...r.seats.map((s) => ({
        id: s.id, npub: s.verified ? s.pk : undefined, verified: s.verified, speaking: s.verified && s.speaking, audio: true,
        audio_mid: `a${s.id}`, video_mid: s.mid ? `v${s.id}` : undefined, me: false,
      })),
    ];
  }

  function view(r: Room): GroupCallView {
    return {
      call_id: r.call_id, group_id: r.group_id, chat_id: chatOf(r.group_id), phase: r.phase, media: r.media, muted: r.muted,
      video_local: r.video_local, camera: r.camera, started_by: r.started_by, started_at: r.started_at, joined_at: r.joined_at, node: r.node, home: r.home,
      participant: r.mine, epoch: r.epoch, participants: r.phase === 'starting' ? [] : participants(r),
      limits: { turn_lifetime_secs: 600, turn_kbps_per_allocation: 2000, credentials_ttl_secs: 600 },
      kbps_per_participant: 2500, max_participants: 12,
    };
  }

  function announced(r: Room): GroupCallAnnounced {
    const people = r.seats.filter((s) => s.verified).map((s) => s.pk);
    return {
      call_id: r.call_id, group_id: r.group_id, chat_id: chatOf(r.group_id), media: r.media, started_by: r.started_by,
      started_at: r.started_at, participants: r.joined ? [host.me(), ...people] : people, joined: r.joined,
    };
  }

  const emitState = () => { if (room?.joined) host.emit('group_call.state', { call: view(room) }); };
  const emitAnnounced = () => { if (room) host.emit('group_call.started', { call: announced(room) }); };

  /** The line of the call in the group's chat, as the runtime writes it (feed.rs of messenger-calls::group). */
  function line(r: Room, outcome: 'ended' | 'failed' | null) {
    const chatId = chatOf(r.group_id);
    const id = `sys:call:${r.call_id}`;
    const list = host.lines(chatId);
    const media = {
      kind: 'group', call_id: r.call_id, direction: r.started_by === host.me() ? 'out' : 'in', media: r.media, started_by: r.started_by,
      participants: r.seen.size, outcome, duration_secs: outcome ? nowSecs() - r.started_at : null, started_at: r.started_at,
    };
    const old = list.find((m) => m.id === id);
    if (old) {
      old.media = media;
      host.emit('dm.updated', { chat_id: chatId, message_id: id });
      return;
    }
    const m: MessengerMessage = {
      id, chat_id: chatId, direction: media.direction as 'in' | 'out', status: 'sent', content_type: 'system', text: 'call', sender_pubkey: r.started_by,
      reply_to: null, created_at: r.started_at, edited_at: null, deleted: false, failure_reason: null, media,
      delivered_at: null, read_at: null, seen_by: [], reactions: [],
    };
    list.push(m);
    host.touch(chatId, r.started_at);
    host.emit('dm.message', { chat_id: chatId, message: m, historical: true });
  }

  /** A member comes into the room: nameless at first, a person once their word is checked. */
  function come(pk?: string, video = false) {
    const r = room;
    if (!r) return;
    const inside = new Set([...r.seats.map((s) => s.pk), ...(r.joined ? [host.me()] : [])]);
    const who = pk ?? host.members(r.group_id).find((m) => !inside.has(m));
    if (!who) return;
    const seat: Seat = { id: nextSeat(r), pk: who, verified: !r.joined, video, mid: true, speaking: false, layer: 'h' };
    r.seats.push(seat);
    emitState();
    if (r.joined) {
      later(1300, () => {
        if (room !== r || !r.seats.includes(seat)) return;
        seat.verified = true;
        r.seen.add(who);
        emitState();
        emitAnnounced();
        line(r, null);
      });
    } else {
      r.seen.add(who);
      emitAnnounced();
      line(r, null);
    }
  }

  /** One leaves; the epoch turns for those who stay. */
  function go(pk?: string) {
    const r = room;
    if (!r || !r.seats.length) return;
    const seat = pk ? r.seats.find((s) => s.pk === pk) : r.seats[r.seats.length - 1];
    if (!seat) return;
    r.seats = r.seats.filter((s) => s !== seat);
    r.epoch += 1;
    if (!r.seats.length && !r.joined) { end(); return; }
    emitState();
    emitAnnounced();
  }

  function end() {
    const r = room;
    if (!r) return;
    stopAll();
    if (r.joined) {
      r.phase = 'left';
      host.emit('group_call.state', { call: { ...view(r), participants: [] } });
      frames.endAll();
    }
    room = null;
    line(r, 'ended');
    host.emit('group_call.ended', { call: { ...announced(r), joined: false }, outcome: 'ended', duration_secs: nowSecs() - r.started_at });
  }

  /**
   * The room's node goes: the way is lost a moment, then the room is on
   * another node (the nearest of mine, so straight on it) under the same
   * call, a new epoch, every seat numbered anew; the group hears of it again.
   */
  function move() {
    const r = room;
    if (!r?.joined || r.phase !== 'in_room') return;
    r.phase = 'reconnecting';
    emitState();
    later(1500, () => {
      if (room !== r || r.phase !== 'reconnecting') return;
      r.home = r.home === NODE ? NODE_2 : NODE;
      r.node = r.home;
      r.mine = 1;
      r.seats.forEach((s, i) => { s.id = i + 2; });
      r.epoch += 1;
      r.phase = 'joining';
      emitState();
      emitAnnounced();
      later(1800, () => { if (room === r && r.phase === 'joining') { r.phase = 'in_room'; emitState(); } });
    });
  }

  /** In the room: the others speak in turns, their voices come as levels. */
  function talk() {
    if (pulse) clearInterval(pulse);
    let t = 0;
    pulse = setInterval(() => {
      const r = room;
      if (!r?.joined) return;
      t += 1;
      const people = r.seats.filter((s) => s.verified);
      const turn = people.length ? people[Math.floor(t / 22) % people.length] : null;
      let changed = false;
      for (const s of r.seats) {
        const now = s === turn && Math.sin(t / 3) > -0.6;
        if (s.speaking !== now) { s.speaking = now; changed = true; }
        if (!s.verified) continue;
        const level = s.speaking ? 0.3 + 0.6 * Math.abs(Math.sin(t * 1.7)) * Math.random() : 0.02 + Math.random() * 0.03;
        host.emit('group_call.level', { call_id: r.call_id, participant: s.id, level });
      }
      if (changed) emitState();
    }, 200);
  }

  function newRoom(groupId: string, media: CallMedia, by: string, ago = 0): Room {
    return {
      call_id: hexId(), group_id: groupId, media, started_by: by, started_at: nowSecs() - ago, seats: [], joined: false, mine: 0,
      phase: 'joining', muted: false, video_local: false, seen: new Set([by]), epoch: 1, node: NODE, home: NODE,
    };
  }

  /** The first group I am in, with people other than me. */
  function someGroup(): string | null {
    for (const id of ['7a', '8b']) {
      const g = id.repeat(64).slice(0, 64);
      if (host.members(g).length > 1) return g;
    }
    return null;
  }

  /** A call in a group on before I came: Boris (or whoever) and one more. */
  function announce(media: CallMedia) {
    if (room) return;
    const g = someGroup();
    if (!g) return;
    const others = host.members(g).filter((m) => m !== host.me());
    const r = newRoom(g, media, others[0], 190);
    room = r;
    r.seats.push({ id: 1, pk: others[0], verified: true, video: media === 'video', mid: true, speaking: false, layer: 'h' });
    if (others[1]) r.seats.push({ id: 2, pk: others[1], verified: true, video: false, mid: true, speaking: false, layer: 'h' });
    for (const s of r.seats) r.seen.add(s.pk);
    line(r, null);
    emitAnnounced();
  }

  /** Into the room on in the group. */
  function enter(r: Room) {
    r.joined = true;
    r.mine = nextSeat(r);
    r.phase = 'joining';
    r.joined_at = nowSecs();
    r.video_local = r.media === 'video';
    r.seen.add(host.me());
    emitState();
    later(800, () => {
      if (room !== r || !r.joined) return;
      r.phase = 'in_room';
      emitState();
      emitAnnounced();
      line(r, null);
      talk();
    });
  }

  function play(kind: string) {
    switch (kind) {
      case 'announce': announce('audio'); break;
      case 'announce-video': announce('video'); break;
      case 'in':
        announce('video');
        if (room && !room.joined && !frames.dmBusy()) {
          enter(room);
          later(2500, () => come());
        }
        break;
      case 'come': come(undefined, Math.random() > 0.5); break;
      case 'go': go(); break;
      case 'video': {
        const s = room?.seats[0];
        if (s) { s.video = !s.video; s.mid = true; emitState(); }
        break;
      }
      case 'video-gone': {
        const s = room?.seats[0];
        if (s) { s.video = !s.video; s.mid = s.video; emitState(); }
        break;
      }
      case 'pause':
      case 'blink': {
        const s = room?.seats.find((x) => x.video);
        if (s) s.pausedUntil = Date.now() + (kind === 'pause' ? 15_000 : 2000);
        break;
      }
      case 'reconnect': {
        const r = room;
        if (!r?.joined || r.phase !== 'in_room') break;
        r.phase = 'reconnecting';
        emitState();
        later(2500, () => { if (room === r && r.phase === 'reconnecting') { r.phase = 'in_room'; emitState(); } });
        break;
      }
      case 'cascade': {
        const r = room;
        if (!r?.joined) break;
        r.node = r.node === r.home ? (r.home === NODE ? NODE_2 : NODE) : r.home;
        emitState();
        break;
      }
      case 'move': move(); break;
      case 'end': end(); break;
    }
  }

  if (host.demo && typeof window !== 'undefined') {
    (window as unknown as { veydanDemoGroupCall?: (kind: string) => void }).veydanDemoGroupCall = play;
  }

  const mocks: Record<string, (args?: Record<string, unknown>) => unknown> = {
    messenger_group_call_get_state: (a) => {
      // The call the demo was asked for comes once the screen has looked.
      if (!asked && host.demo) {
        asked = true;
        let kind: string | null = null;
        try { kind = localStorage.getItem('messenger.demo.gcall'); } catch { /* no storage */ }
        const play1 = kind === 'announced' ? 'announce' : kind === 'announced-video' ? 'announce-video' : kind === 'in' ? 'in' : null;
        if (play1) setTimeout(() => play(play1), 1200);
      }
      const groupId = (a?.groupId as string | null) ?? null;
      return {
        call: room?.joined ? view(room) : null,
        announced: room && groupId && room.group_id === groupId ? announced(room) : null,
      };
    },
    messenger_group_call_start: (a) => {
      const groupId = String(a?.groupId ?? '');
      const media = (a?.media as CallMedia) ?? 'audio';
      if (frames.dmBusy()) throw refuse('a call is under way');
      if (room?.joined) throw refuse('a group call is under way');
      if (room?.group_id === groupId) throw refuse('a call is on in this group already: join it');
      if (!host.members(groupId).includes(host.me())) throw refuse('not a member of the group');
      // A call of another group that I am not in plays no further.
      if (room) { stopAll(); room = null; }
      const r = newRoom(groupId, media, host.me());
      room = r;
      r.joined = true;
      r.mine = 1;
      r.phase = 'starting';
      r.video_local = media === 'video';
      emitState();
      later(600, () => {
        if (room !== r) return;
        r.phase = 'joining';
        r.joined_at = nowSecs();
        line(r, null);
        emitState();
        emitAnnounced();
      });
      later(1400, () => { if (room === r) { r.phase = 'in_room'; emitState(); talk(); } });
      later(3000, () => come());
      later(6500, () => come(undefined, true));
      return { ...view(r), phase: 'starting' };
    },
    messenger_group_call_join: (a) => {
      const groupId = String(a?.groupId ?? '');
      if (frames.dmBusy()) throw refuse('a call is under way');
      if (room?.joined) throw refuse('a group call is under way');
      if (!room || room.group_id !== groupId) throw refuse('no call is on in this group');
      enter(room);
      return view(room);
    },
    messenger_group_call_leave: () => {
      const r = room;
      if (!r?.joined) throw refuse('no group call');
      if (!r.seats.length) { end(); return; }
      stopAll();
      frames.endAll();
      host.emit('group_call.state', { call: { ...view(r), phase: 'left', participants: [] } });
      r.joined = false;
      r.video_local = false;
      emitAnnounced();
      // The others talk on a while, then go too.
      later(20_000, () => { if (room === r && !r.joined) end(); });
    },
    messenger_group_call_mute: (a) => {
      if (!room?.joined) throw refuse('no group call');
      room.muted = Boolean(a?.muted);
      emitState();
      return view(room);
    },
    messenger_group_call_set_video: (a) => {
      if (!room?.joined) throw refuse('no group call');
      const input = (a?.input ?? { kind: 'off' }) as VideoInput;
      room.video_local = input.kind !== 'off';
      emitState();
      return view(room);
    },
    messenger_group_call_switch_camera: (a) => {
      if (!room?.joined) throw refuse('no group call');
      const wanted = typeof a?.camera === 'string' ? a.camera : null;
      room.camera = wanted ?? (room.camera === 'back' ? 'front' : 'back');
      emitState();
      return view(room);
    },
    messenger_group_call_set_layer: (a) => {
      const s = room?.seats.find((x) => x.id === Number(a?.participant));
      if (!s) throw refuse('no such seat');
      s.layer = String(a?.rid ?? 'h') in LAYERS ? String(a?.rid) : 'h';
    },
    messenger_group_call_video_subscribe: (a) => {
      const channel = (a?.channel ?? {}) as DemoChannel;
      const r = room;
      if (!r?.joined) return frames.endNow(channel);
      const mid = String(a?.mid ?? '');
      // My own pictures (the tile of "me"): the test pattern of my camera while it goes.
      if (mid === MY_VIDEO_MID) {
        return frames.subscribe(channel, (seq) => (room !== r || !r.video_local ? null : testPattern(640, 360, seq, 0.4)), FPS);
      }
      const id = Number(mid.slice(1));
      return frames.subscribe(channel, (seq) => {
        const s = r.seats.find((x) => x.id === id);
        if (room !== r || !s?.video || !s.mid || !s.verified || Date.now() < (s.pausedUntil ?? 0)) return null;
        const [w, h] = LAYERS[s.layer] ?? LAYERS.h;
        return testPattern(w, h, seq, 1.2 + id * 1.3);
      }, FPS);
    },
  };

  return { mocks, active: () => !!room?.joined };
}

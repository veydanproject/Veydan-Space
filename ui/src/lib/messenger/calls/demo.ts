// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Calls of the browser preview (`pnpm dev` without Tauri), played so that
// every screen of a call can be looked at without a backend. Never used
// inside the app.
//
// A call made in the demo is answered in a few seconds (Boris declines); one
// comes in when asked:
//   localStorage['messenger.demo.call'] = 'incoming' | 'active' | 'relay' | 'outgoing'
//     | 'video' (a video call rings) | 'video-active' | 'video-outgoing'
// plays it a moment after the page loads, and `window.veydanDemoCall(kind)`
// plays it at once (also `'end'`, and `'peer-video'` that turns Alice's
// camera off or on).
//
// Video is a test picture (calls/video.ts `testPattern`) sent to the page's
// channel as the runtime sends frames: the peer's 640×360, mine 640×360 as
// well on a computer and as a phone's front camera gives it (turned, to be
// stood upright by the page) on a narrow screen, my screen 960×540. As the
// runtime does, the next frame goes only once the page acknowledged the
// last one (`messenger_call_video_ack`).

import type { MessengerChat, MessengerMessage } from '../api';
import type {
  CallMedia, CallNodeInput, CallNodeView, CallOutcome, CallState, CallView, RelayPolicy, VideoInput, VideoQuality, VideoTrack,
} from '../generated/calls';
import { HEADER_BYTES, testPattern } from './video';

export interface DemoCallHost {
  /** The demo's data is on (`messenger.demo=1`). */
  demo: boolean;
  emit: (name: string, payload: unknown) => void;
  chat: (peer: string) => MessengerChat;
  /** The messages of a chat, to add a call's line to. */
  lines: (chatId: string) => MessengerMessage[];
  /** The chat list shows the call. */
  touch: (chatId: string, at: number) => void;
}

const ALICE = '1a'.repeat(32);
const BORIS = '2b'.repeat(32);
const NODE = '108.61.171.68:8443#fda09da75199c4e04601a9df309710fab15cd7b2b806ca3b2bbab202582a6dca';
const REF = /^[^\s#]+:\d{1,5}#[0-9a-fA-F]{64}$/;

const CAMERAS = [{ id: '/dev/video0', name: 'Integrated Camera' }, { id: '/dev/video2', name: 'USB Camera' }];
const SCREENS = [
  { id: 'screen:0', title: 'Screen 1', window: false },
  { id: 'window:4194307', title: 'Veydan — notes on calls', window: true },
  { id: 'window:6291462', title: 'Terminal', window: true },
];
const FPS = 24;

const nowSecs = () => Math.floor(Date.now() / 1000);
/** A phone of the demo: a narrow page. Its own camera gives its frames turned. */
const narrow = () => typeof window !== 'undefined' && window.innerWidth < 600;
const hexId = () => Array.from({ length: 32 }, () => Math.floor(Math.random() * 16).toString(16)).join('');

export function demoCallMocks(host: DemoCallHost): Record<string, (args?: Record<string, unknown>) => unknown> {
  let call: CallView | null = null;
  let policy: RelayPolicy = 'auto';
  let own: { reference: string; key: boolean }[] = [];
  const timers: ReturnType<typeof setTimeout>[] = [];
  let pulse: ReturnType<typeof setInterval> | null = null;
  let asked = false;
  /** The phone of the demo: its earpiece and loudspeaker. */
  let route: 'earpiece' | 'speaker' = 'earpiece';
  let quality: VideoQuality = '360p';
  /**
   * The page's channels, by subscription: each gets the test picture of its
   * track. `sent`: the `seq` of the frame the page has not acknowledged yet;
   * no other goes until it has.
   */
  const subs = new Map<number, {
    track: VideoTrack; channel: { onmessage?: (data: ArrayBuffer) => void }; seq: number; sent: number | null; timer: ReturnType<typeof setInterval>;
  }>();
  let nextSub = 1;

  const nodes = (): CallNodeView[] => [
    ...own.map((n) => ({ reference: n.reference, id: n.reference.split('#')[1].toLowerCase(), class: 'own', has_key: n.key })),
    ...(own.some((n) => n.reference === NODE) ? [] : [{ reference: NODE, id: NODE.split('#')[1], class: 'project', has_key: false }]),
  ];
  const state = (): CallState => ({ call, policy, nodes: nodes(), available: true, video_quality: quality });
  const later = (ms: number, fn: () => void) => timers.push(setTimeout(fn, ms));
  const stopAll = () => {
    while (timers.length) clearTimeout(timers.pop());
    if (pulse) clearInterval(pulse);
    pulse = null;
  };

  function line(view: CallView, outcome: CallOutcome | null, duration: number | null) {
    const id = `sys:call:${view.call_id}`;
    const list = host.lines(view.chat_id);
    const media = {
      call_id: view.call_id, direction: view.direction, media: view.media, outcome,
      duration_secs: duration, via: outcome ? (view.via ?? 'direct') : null, started_at: view.started_at,
    };
    const old = list.find((m) => m.id === id);
    if (old) {
      old.media = media;
      host.emit('dm.updated', { chat_id: view.chat_id, message_id: id });
      return;
    }
    const m: MessengerMessage = {
      id, chat_id: view.chat_id, direction: view.direction, status: 'sent', content_type: 'system', text: 'call', sender_pubkey: '',
      reply_to: null, created_at: view.started_at, edited_at: null, deleted: false, failure_reason: null, media,
      delivered_at: null, read_at: null, seen_by: [], reactions: [],
    };
    list.push(m);
    host.touch(view.chat_id, view.started_at);
    host.emit('dm.message', { chat_id: view.chat_id, message: m, historical: true });
  }

  function put(view: CallView, event = 'call.state') {
    call = sized(view);
    host.emit(event, { call });
  }

  /** The sizes of the videos as the runtime tells them: there while a video goes. */
  function sized(view: CallView): CallView {
    const mine = view.video_screen ? { width: 960, height: 540 } : narrow() ? { width: 360, height: 640 } : { width: 640, height: 360 };
    return {
      ...view,
      video_local_size: view.video_local ? mine : undefined,
      video_remote_size: view.video_remote && view.phase === 'active' ? { width: 640, height: 360 } : undefined,
    };
  }

  /** One frame of a track, or nothing while that video does not go. */
  function frame(track: VideoTrack, seq: number): ArrayBuffer | null {
    const c = call;
    if (!c || c.phase === 'incoming') return null;
    if (track === 'remote') return c.video_remote && c.phase === 'active' ? testPattern(640, 360, seq, 2.2) : null;
    if (!c.video_local) return null;
    if (c.video_screen) return testPattern(960, 540, seq, 0.6);
    const back = c.camera === 'back' || c.camera === CAMERAS[1].id;
    return narrow() ? testPattern(360, 640, seq, back ? 4.4 : 5.4, 270) : testPattern(640, 360, seq, back ? 4.4 : 5.4);
  }

  function unsubscribe(id: number, last: boolean) {
    const s = subs.get(id);
    if (!s) return;
    clearInterval(s.timer);
    subs.delete(id);
    if (last) s.channel.onmessage?.(new ArrayBuffer(HEADER_BYTES));
  }

  /** Talking: the clock runs, the engine tells its numbers, the peer speaks in bursts. */
  function talk(via: 'direct' | 'relay') {
    if (!call) return;
    put({ ...call, phase: 'active', via, answered_at: call.answered_at ?? nowSecs() });
    let t = 0;
    pulse = setInterval(() => {
      if (!call) return;
      t += 1;
      const speaking = Math.sin(t / 9) > -0.2;
      host.emit('call.level', { call_id: call.call_id, level: speaking ? 0.25 + 0.6 * Math.abs(Math.sin(t * 1.7)) * Math.random() : 0.02 });
      if (t % 10 === 1) {
        const relay = call.via === 'relay';
        host.emit('call.stats', {
          call_id: call.call_id,
          stats: { rtt_ms: (relay ? 92 : 38) + Math.round(Math.random() * 14), packets_lost: Math.floor(t / 40), jitter_ms: 3 + Math.round(Math.random() * 4), bytes_sent: t * 4200, bytes_received: t * 4100 },
        });
      }
    }, 200);
  }

  function finish(outcome: CallOutcome) {
    const c = call;
    if (!c) return;
    stopAll();
    route = 'earpiece';
    const duration = c.answered_at ? nowSecs() - c.answered_at : null;
    call = null;
    for (const id of [...subs.keys()]) unsubscribe(id, true);
    line(c, outcome, duration);
    host.emit('call.ended', { call: { ...c, phase: 'ended' }, outcome, duration_secs: duration });
  }

  function newCall(peer: string, direction: 'in' | 'out', media: CallMedia): CallView {
    const chat = host.chat(peer);
    return {
      call_id: hexId(), peer: chat.peer_pubkey ?? peer, chat_id: chat.id, direction, media, phase: direction === 'in' ? 'incoming' : 'outgoing',
      muted: false, started_at: nowSecs(), nodes: [NODE.split('#')[1]],
      // A video call: the caller's camera is on from the start, the peer's once it answers.
      video_local: media === 'video' && direction === 'out', video_screen: false, video_remote: media === 'video' && direction === 'in',
      limits: { turn_lifetime_secs: 600, turn_kbps_per_allocation: 2000, credentials_ttl_secs: 600 },
    };
  }

  /** A call from Alice: ringing, or already answered (on "another screen", as the phone's Answer does). */
  function play(kind: string) {
    if (kind === 'end') { finish(call?.answered_at ? 'ended' : 'missed'); return; }
    if (kind === 'peer-video') { if (call) put({ ...call, video_remote: !call.video_remote }); return; }
    if (call) return;
    if (kind === 'outgoing' || kind === 'video-outgoing') { start({ peer: ALICE, media: kind === 'outgoing' ? 'audio' : 'video' }, true); return; }
    const media: CallMedia = kind.startsWith('video') ? 'video' : 'audio';
    const view = newCall(ALICE, 'in', media);
    line(view, null, null);
    put(view, 'call.incoming');
    if (kind === 'incoming' || kind === 'video') {
      later(45_000, () => { if (call?.call_id === view.call_id && call.phase === 'incoming') finish('missed'); });
      return;
    }
    put({ ...view, phase: 'connecting', answered_at: nowSecs() - 83, video_local: media === 'video' });
    talk(kind === 'relay' ? 'relay' : 'direct');
  }

  /** `hold`: the peer never answers (the screen of a call that rings). */
  function start(a: Record<string, unknown> | undefined, hold = false): CallView {
    if (call) throw { code: 'other', message: 'a call is under way' };
    const chat = host.chat(String(a?.peer ?? ''));
    if (chat.mode !== 'full_chat') throw { code: 'other', message: 'calls go to contacts only' };
    const view = newCall(chat.peer_pubkey ?? '', 'out', (a?.media as CallMedia) ?? 'audio');
    line(view, null, null);
    put(view);
    if (hold) return view;
    if (view.peer === BORIS) {
      later(4000, () => { if (call?.call_id === view.call_id) finish('declined'); });
      return view;
    }
    later(3500, () => {
      if (call?.call_id !== view.call_id) return;
      put({ ...call, phase: 'connecting', answered_at: nowSecs(), video_remote: call.video_remote || view.media === 'video' });
      later(1200, () => { if (call?.call_id === view.call_id) talk(policy === 'relay_only' ? 'relay' : 'direct'); });
    });
    return view;
  }

  if (host.demo && typeof window !== 'undefined') {
    (window as unknown as { veydanDemoCall?: (kind: string) => void }).veydanDemoCall = play;
  }

  return {
    messenger_call_get_state: () => {
      // The call the demo was asked for comes once the screen has looked.
      if (!asked && host.demo) {
        asked = true;
        let kind: string | null = null;
        try { kind = localStorage.getItem('messenger.demo.call'); } catch { /* no storage */ }
        if (kind) setTimeout(() => play(kind), 1500);
      }
      return state();
    },
    messenger_call_start: (a) => start(a),
    messenger_call_accept: (a) => {
      if (!call || call.call_id !== a?.callId || call.phase !== 'incoming') throw { code: 'other', message: 'no such call is ringing' };
      // Taking a video call turns my camera on, as the runtime does.
      put({ ...call, phase: 'connecting', answered_at: nowSecs(), video_local: call.media === 'video' });
      const id = call.call_id;
      later(900, () => { if (call?.call_id === id) talk(policy === 'relay_only' ? 'relay' : 'direct'); });
      return call;
    },
    messenger_call_decline: (a) => { if (call?.call_id === a?.callId) finish('declined'); },
    messenger_call_end: (a) => {
      if (!call || call.call_id !== a?.callId) throw { code: 'other', message: 'no such call' };
      finish(call.answered_at ? 'ended' : 'missed');
    },
    messenger_call_mute: (a) => {
      if (!call) throw { code: 'other', message: 'no call' };
      put({ ...call, muted: Boolean(a?.muted) });
      return call;
    },
    messenger_call_audio_route: (a) => {
      if (!call) throw { code: 'other', message: 'no call holds the sound' };
      const input = (a?.input ?? {}) as { op?: string; route?: string };
      if (input.op === 'set') route = input.route === 'speaker' ? 'speaker' : 'earpiece';
      return { current: route, available: ['earpiece', 'speaker'] };
    },
    messenger_call_set_video: (a) => {
      if (!call) throw { code: 'other', message: 'no call' };
      const input = (a?.input ?? { kind: 'off' }) as VideoInput;
      const on = input.kind !== 'off';
      const camera = input.kind === 'camera' ? (input.id ?? call.camera) : call.camera;
      put({ ...call, video_local: on, video_screen: input.kind === 'screen', camera });
      // Alice answers my camera with hers, a moment later.
      if (on && !call.video_remote) {
        const id = call.call_id;
        later(1500, () => { if (call?.call_id === id && call.video_local && !call.video_remote) put({ ...call, video_remote: true }); });
      }
      return call;
    },
    messenger_call_switch_camera: (a) => {
      if (!call) throw { code: 'other', message: 'no call' };
      const ids = narrow() ? ['front', 'back'] : CAMERAS.map((c) => c.id);
      const next = (a?.camera as string | null) ?? ids[(ids.indexOf(call.camera ?? ids[0]) + 1) % ids.length];
      put({ ...call, camera: next });
      return call;
    },
    messenger_call_list_cameras: () => (narrow() ? [] : CAMERAS),
    messenger_call_list_screens: () => SCREENS,
    messenger_call_share_screen: () => {
      if (!call) throw { code: 'other', message: 'no call' };
      put({ ...call, video_local: true, video_screen: true });
      return call;
    },
    messenger_call_set_video_quality: (a) => { quality = a?.quality === '720p' ? '720p' : '360p'; return state(); },
    messenger_call_video_subscribe: (a) => {
      const id = nextSub++;
      const channel = (a?.channel ?? {}) as { onmessage?: (data: ArrayBuffer) => void };
      const track: VideoTrack = a?.track === 'local' ? 'local' : 'remote';
      if (!call) {
        setTimeout(() => channel.onmessage?.(new ArrayBuffer(HEADER_BYTES)), 0);
        return id;
      }
      const sub = {
        track, channel, seq: 0, sent: null as number | null,
        timer: setInterval(() => {
          if (sub.sent != null) return;
          const f = frame(sub.track, sub.seq);
          if (!f) return;
          sub.sent = sub.seq;
          sub.seq += 1;
          sub.channel.onmessage?.(f);
        }, 1000 / FPS),
      };
      subs.set(id, sub);
      return id;
    },
    // The page took the frame `seq` (or a later one): the next may go.
    messenger_call_video_ack: (a) => {
      const s = subs.get(Number(a?.id));
      if (s && s.sent != null && Number(a?.seq) >= s.sent) s.sent = null;
    },
    messenger_call_video_unsubscribe: (a) => { unsubscribe(Number(a?.id), false); },
    messenger_call_set_policy: (a) => { policy = a?.policy === 'relay_only' ? 'relay_only' : 'auto'; return state(); },
    messenger_call_set_nodes: (a) => {
      const list = (a?.nodes ?? []) as CallNodeInput[];
      const bad = list.find((n) => !REF.test(n.reference.trim()));
      if (bad) throw { code: 'other', message: `invalid input: not a node reference (address:port#id): ${bad.reference}` };
      own = list.map((n) => ({ reference: n.reference.trim(), key: !!n.key || own.find((o) => o.reference === n.reference.trim())?.key === true }));
      return state();
    },
  };
}

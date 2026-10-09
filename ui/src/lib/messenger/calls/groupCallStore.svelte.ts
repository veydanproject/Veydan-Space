// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The call of a group: the room I am in, and the calls that are on in my
// groups (the banner of a group's chat). Fed by the runtime's
// `group_call.*` events, which the module store forwards here; the room's
// screens (the desk's window and capsule, the phone's page) and the
// group's header, banner and lines read it.
//
// One call of either kind at a time: the runtime refuses a group call
// while a call of two is under way and the other way round. A room I left
// (or that went) stays as `over` for a moment, so the screen can say how.

import {
  messengerApi,
  type CallAudioRoute,
  type CallAudioRoutes,
  type CallMedia,
  type GroupCallAnnounced,
  type GroupCallView,
  type GroupParticipant,
  type MessengerUiEvent,
  type VideoInput,
} from '../api';
import { focusOf, groupStatusText, holdFocus, seatSendsVideo, seatVideoWord, FOCUS_HOLD_MS, type GroupOver, type Layer } from './group';
import { VIDEO_LOST_MS } from './video';

/** How long the screen shows how a group call ended for me. */
export const GROUP_OVER_SHOWN_MS = 2500;

/** How long a failure the room told of (not a command's refusal) stays on the screen. */
export const ERROR_SHOWN_MS = 6000;

/**
 * A seat's pictures come this moment (`flowing`): one was drawn this long
 * ago or less. A tile made anew waits dark for its next picture then; it
 * shows the face otherwise.
 */
export const PICTURE_FLOWING_MS = 1500;

/**
 * How long a word of state carried over a rejoin (the way restored, the
 * room moved) stands for a confirmed seat whose own word has not come: the
 * word follows the word of identity at once (messenger-wire §10), so a
 * seat that says none by then says none at all (an older client on that
 * device), and is judged by its frames.
 */
export const CARRIED_WORD_MS = 10_000;

type Word = Pick<GroupParticipant, 'camera' | 'mic' | 'screen'>;

export interface GroupCallOver {
  /** The room as it was last seen (its seats included). */
  call: GroupCallView;
  how: GroupOver;
}

/** How far the room has come: an answer older than an event does not take it back. */
const PHASE_ORDER: Record<GroupCallView['phase'], number> = { starting: 0, joining: 1, in_room: 2, reconnecting: 2, left: 3 };

/** The node of the room (`home`; the node I am on when a runtime without the cascade says none). */
const homeOf = (view: Pick<GroupCallView, 'node' | 'home'>) => view.home || view.node;

class GroupCallStore {
  /** The room I am in; `null` when none. */
  call = $state<GroupCallView | null>(null);
  /** The room I just left, or that went, for a moment. */
  over = $state<GroupCallOver | null>(null);
  /** The calls that are on in my groups, by group: the banners. */
  announced = $state<Record<string, GroupCallAnnounced>>({});
  /** How loud each seat of my room is, 0..1, by seat. */
  levels = $state<Record<number, number>>({});
  /** The seat that spoke last (not mine), for the large tile. */
  speaker = $state<number | null>(null);
  /** The clock of the room, unix seconds, ticking while I am in one. */
  now = $state(Math.floor(Date.now() / 1000));
  /** A command of the room is on its way. */
  busy = $state(false);
  /** Leaving is on its way; kept apart from `busy` so that Leave works while another command has not come back. */
  ending = $state(false);
  /** The last failure, as the runtime said it; the screen words it. */
  error = $state<unknown>(null);
  /** What my video sends is my screen (the runtime says only that video goes). */
  screen = $state(false);
  /** A computer: the room's window is open, not folded into its capsule. Open again for every new room. */
  shown = $state(true);
  /**
   * The seats whose pictures come, by seat: for the order of the screen
   * where a seat says nothing of its camera (an older client: calls/group.ts
   * `seatSendsVideo`; a seat that says it is believed, `seatVideoWord`;
   * a tile made anew goes by `flowing` and `stale`). On with the first picture a tile drew of it
   * (`seatPicture`), off once its pictures have been missing for
   * `VIDEO_LOST_MS` or their stream ended (`seatEnded`). The clock of each
   * seat is here, not in its tile: a tile that moves (between the large
   * place, the row and the grid) or comes again (the window unfolded) is
   * made anew, and a camera that is off would count from naught with each.
   * A shorter pause of the frames moves nothing: the tile keeps its last
   * picture. Let go with the seat, and with the m-line of its video.
   */
  showing = $state<Record<number, boolean>>({});
  /**
   * The seats a picture of which was drawn a moment ago
   * (`PICTURE_FLOWING_MS`), by seat: a tile made anew of such a seat waits
   * dark for the next one, which is on its way; of another, it shows the
   * face at once (calls/group.ts `tileWait`).
   */
  flowing = $state<Record<number, boolean>>({});
  /**
   * The seats whose camera (or screen) is on by their own word and whose
   * pictures have been missing for `VIDEO_LOST_MS` (since the last one, or
   * since the word said on when none came after), by seat: "No signal" by
   * the seat's clock, not by that of a tile made anew (calls/group.ts
   * `tileWait`). Nothing of the order of the screen follows it.
   */
  stale = $state<Record<number, boolean>>({});
  /**
   * The seat the voices give the large place (not a tap: the screen keeps
   * that), held against a quick change (calls/group.ts `holdFocus`).
   */
  voice = $state<number | null>(null);
  /** A phone: where the sound of the room can go and where it goes; `null` on a computer, or before the room holds the sound. */
  routes = $state<CallAudioRoutes | null>(null);
  /**
   * The room is moving to another node: its node went, and the room I am
   * entering again has another home than the one I talked in. The runtime
   * says `reconnecting`/`joining` under the same call; the screen stays and
   * says so until I am in the room again.
   */
  moving = $state(false);

  /** The home of the room I last talked in (`in_room`), to tell a move from a way merely lost. */
  private settledHome: string | null = null;
  /**
   * The room changed under the same call on the same node: a double move
   * (cascade.md «Переезд» п.4) can put the loser's room and the winner's on
   * one node, so the home alone does not tell it. Until I am in the room.
   */
  private anotherRoom = false;
  /** My camera was on when my screen went on: stopping the screen brings it back. */
  private cameraBeforeScreen = false;
  /** The room this page is leaving: its end is mine. */
  private leaving: string | null = null;
  private overTimer: ReturnType<typeof setTimeout> | null = null;
  private ticker: ReturnType<typeof setInterval> | null = null;
  /** Counts every change of the room: a snapshot asked before one and come after it is older. */
  private epoch = 0;
  /** The same for the banners. */
  private announcedEpoch = 0;
  /**
   * The rooms that are over here: a late word of one is no new room. A
   * call keeps its id across joins, so a start or a join in a group lets
   * the group's go again (`enter`).
   */
  private finished: { callId: string; groupId: string }[] = [];
  /** A start or a join is on its way: a room that fails before I am in it is told by its refusal, not as a room over. */
  private entering = 0;
  /** The layer last asked of each seat's video; a node without simulcast is asked none. */
  private layers = new Map<number, Layer>();
  private simulcast = true;
  private errorTimer: ReturnType<typeof setTimeout> | null = null;
  /** Since when `voice` has its seat (`Date.now()`). */
  private voiceSince = 0;
  /** A look again at the large place once its hold is over. */
  private voiceTimer: ReturnType<typeof setTimeout> | null = null;
  /** When a tile last drew a picture of each seat taken as on (`Date.now()`). */
  private pictureAt = new Map<number, number>();
  /** When a tile last drew a picture of each seat, taken as on or not (`Date.now()`): `flowing`, `stale`. */
  private lastPicture = new Map<number, number>();
  /** Since when each seat says its camera (or screen) is on (`Date.now()`): `stale`. */
  private toldAt = new Map<number, number>();
  /**
   * The last word of state of each person with one seat in the room (by
   * npub): what a rejoin carries over (`carried`). A person in the room
   * from two devices at once has two seats, each with its own word or
   * none (an older client): theirs is not kept, as nothing tells which
   * device said it. Let go with the room, and with a person who left it.
   */
  private words = new Map<string, Word>();
  /**
   * The words carried over the last rejoin (the way restored, the room
   * moved: the runtime knows the seats anew, their words follow their
   * words of identity), by npub: given to the one confirmed seat of the
   * person until its own word comes, for `CARRIED_WORD_MS` at most, so the
   * screen does not fall back to the frames for that moment. Never
   * outside a rejoin, and never to a person with two seats.
   */
  private carried = new Map<string, { word: Word; until: number | null }>();
  /** The room as the runtime said it last, before `carried` filled it: looked at again when a carried word lapses. */
  private raw: GroupCallView | null = null;

  /** The room I am in when it is the call of `groupId`. */
  inGroup(groupId: string | null | undefined): GroupCallView | null {
    return groupId && this.call?.group_id === groupId ? this.call : null;
  }

  /** The call on in `groupId`, for its banner. */
  announcedIn(groupId: string | null | undefined): GroupCallAnnounced | null {
    return groupId ? (this.announced[groupId] ?? null) : null;
  }

  /**
   * What the room I am in says of itself in a line (the clock, a phase, or
   * that it moves): for every place that shows the room going, so that the
   * chat's banner, the phone's bar and the capsule say the same.
   */
  statusText(tr: (key: string, params?: Record<string, string>) => string): string {
    return groupStatusText(this.call, null, this.now, tr, !!this.call && this.moving);
  }

  /** The room has video to show: mine goes, or the pictures of a seat come. */
  get video(): boolean {
    const c = this.call;
    return !!c && (c.video_local || c.participants.some((p) => !p.me && seatSendsVideo(p, c, (seat) => !!this.showing[seat])));
  }

  /** The room I am in, and (with `groupId`) the call on in that group, as the runtime has them. */
  async load(groupId?: string) {
    const at = this.epoch;
    const atAnnounced = this.announcedEpoch;
    const s = await messengerApi.groupCalls.state(groupId);
    if (groupId && this.announcedEpoch === atAnnounced) {
      if (s.announced) this.announced = { ...this.announced, [groupId]: s.announced };
      else if (this.announced[groupId]) this.dropAnnounced(groupId);
    }
    if (this.epoch !== at) return;
    const live = s.call && s.call.phase !== 'left' && !this.isFinished(s.call.call_id) ? s.call : null;
    if (live) this.setCall(live);
    else if (this.call) this.setCall(null);
  }

  /** A call in the group: a room on a node, me in it, the group told. The refusal is handed back for the button that shows it. */
  start(groupId: string, media: CallMedia = 'audio') {
    return this.enter(groupId, () => messengerApi.groupCalls.start(groupId, media));
  }

  /** Into the call on in the group, again after I left it too. The refusal is handed back for the banner that shows it. */
  join(groupId: string) {
    return this.enter(groupId, () => messengerApi.groupCalls.join(groupId));
  }

  private async enter(groupId: string, fn: () => Promise<GroupCallView>): Promise<{ view: GroupCallView | null; refusal: unknown }> {
    this.busy = true;
    this.error = null;
    // The rooms of this group that were over here are entered anew: the
    // call keeps its id from one join to the next.
    this.finished = this.finished.filter((f) => f.groupId !== groupId);
    this.entering++;
    try {
      return { view: this.adopt(await fn()), refusal: null };
    } catch (e) {
      return { view: null, refusal: e };
    } finally {
      this.entering--;
      this.busy = false;
    }
  }

  /** Out of the room; the last one out ends the call for the group. */
  async leave() {
    const c = this.call;
    if (!c) return;
    this.leaving = c.call_id;
    this.ending = true;
    this.error = null;
    try {
      await messengerApi.groupCalls.leave();
      // The runtime says `group_call.state` (left) too; the screen goes at once.
      this.finish(c, true);
    } catch (e) {
      this.error = e;
      this.leaving = null;
      await this.load().catch(() => {});
    } finally {
      this.ending = false;
    }
  }

  async toggleMute() {
    const c = this.call;
    if (!c) return;
    await this.run(async () => this.take(await messengerApi.groupCalls.mute(!c.muted)));
  }

  /** My camera on, or off; a screen being shown gives way to the camera. */
  async toggleCamera() {
    const c = this.call;
    if (!c) return;
    this.cameraBeforeScreen = false;
    const off = c.video_local && !this.screen;
    if (await this.applyVideo(off ? { kind: 'off' } : { kind: 'camera' })) this.screen = false;
  }

  /** My screen (or the window `id`) instead of my camera. */
  async shareScreen(id?: string) {
    const c = this.call;
    if (!c) return;
    const camera = c.video_local && !this.screen;
    if (await this.applyVideo({ kind: 'screen', id })) {
      this.screen = true;
      this.cameraBeforeScreen = camera;
    }
  }

  /** The screen no more: back to the camera when it was on before, off otherwise. */
  async stopScreen() {
    if (!this.call) return;
    const back = this.cameraBeforeScreen;
    this.cameraBeforeScreen = false;
    if (await this.applyVideo(back ? { kind: 'camera' } : { kind: 'off' })) this.screen = false;
  }

  private async applyVideo(input: VideoInput): Promise<boolean> {
    const view = await this.run(async () => this.take(await messengerApi.groupCalls.setVideo(input)));
    return view !== undefined;
  }

  /** The next camera (a phone: front, back): at once while mine is on, for the next time otherwise. */
  async switchCamera() {
    if (!this.call) return;
    await this.run(async () => this.take(await messengerApi.groupCalls.switchCamera()));
  }

  /**
   * The layer of a seat's video its tile needs. Asked once per change; a
   * node without simulcast refuses, and is not asked again in this room.
   */
  async setLayer(seat: number, layer: Layer) {
    const c = this.call;
    if (!c || !this.simulcast || this.layers.get(seat) === layer) return;
    this.layers.set(seat, layer);
    try {
      await messengerApi.groupCalls.setLayer(seat, layer);
    } catch (e) {
      if (/simulcast/i.test(String((e as { message?: unknown })?.message ?? e))) this.simulcast = false;
      // Asked again with the next change of the tile.
      else if (this.call?.call_id === c.call_id) this.layers.delete(seat);
    }
  }

  /** The answer of a command about the room under way, unless another room took its place. */
  private take(view: GroupCallView): GroupCallView {
    if (this.call?.call_id === view.call_id && view.phase !== 'left') this.setCall(view);
    return view;
  }

  /**
   * The answer of a start or a join, unless the store knows better: the
   * room went meanwhile, or an event already took it further.
   */
  private adopt(view: GroupCallView): GroupCallView {
    if (this.isFinished(view.call_id) || view.phase === 'left') return view;
    const c = this.call;
    if (c?.call_id === view.call_id && PHASE_ORDER[c.phase] > PHASE_ORDER[view.phase]) return view;
    this.setCall(view);
    return view;
  }

  /** A command, with the error kept for the screen; `undefined` when it failed. */
  private async run<T>(fn: () => Promise<T>): Promise<T | undefined> {
    this.busy = true;
    this.error = null;
    try {
      return await fn();
    } catch (e) {
      this.error = e;
      return undefined;
    } finally {
      this.busy = false;
    }
  }

  clearError() {
    this.error = null;
  }

  /** A phone's room screen asks once the sound is the room's; a computer refuses, and nothing is shown. */
  async loadRoutes() {
    try {
      this.routes = await messengerApi.calls.audioRoute('list');
    } catch {
      this.routes = null;
    }
  }

  /** The loudspeaker on, or off again: back to a headset when one is there, the earpiece otherwise (as in a call of two). */
  async toggleSpeaker() {
    const r = this.routes;
    if (!r) return;
    const order: CallAudioRoute[] = ['bluetooth', 'wired', 'earpiece'];
    const next = r.current === 'speaker' ? (order.find((x) => r.available.includes(x)) ?? 'earpiece') : 'speaker';
    await this.run(async () => { this.routes = await messengerApi.calls.audioRoute('set', next); });
  }

  /** A failure the room told of, not a command's refusal: shown for a while, then let go. */
  private tell(error: unknown) {
    this.error = error;
    if (this.errorTimer) clearTimeout(this.errorTimer);
    this.errorTimer = setTimeout(() => { if (this.error === error) this.error = null; }, ERROR_SHOWN_MS);
  }

  /**
   * A tile drew a picture of the seat whose video has the m-line `mid`:
   * its camera is on, and its clock starts again. Told by the m-line of
   * the tile's subscription, not by the seat its tile shows now: a tile
   * given another seat says nothing of the new one with words of the old.
   */
  seatPicture(mid: string) {
    const seat = this.seatOf(mid);
    if (seat == null) return;
    const now = Date.now();
    this.pictureAt.set(seat, now);
    this.lastPicture.set(seat, now);
    this.flag('flowing', seat, true);
    this.flag('stale', seat, false);
    if (!this.showing[seat]) this.setShowing(seat, true);
  }

  /** The stream of the seat's video (by its m-line) ended: its camera is off at once. */
  seatEnded(mid: string) {
    const seat = this.seatOf(mid);
    if (seat == null) return;
    this.flag('flowing', seat, false);
    this.setShowing(seat, false);
  }

  /** A flag of a seat (`flowing`, `stale`), written only when it changes. */
  private flag(name: 'flowing' | 'stale', seat: number, on: boolean) {
    if ((this[name][seat] ?? false) === on) return;
    if (on) this[name] = { ...this[name], [seat]: true };
    else {
      const { [seat]: _gone, ...rest } = this[name];
      this[name] = rest;
    }
  }

  /** Another's seat whose video has the m-line `mid`, in the room I am in. */
  private seatOf(mid: string): number | null {
    return this.call?.participants.find((p) => !p.me && p.video_mid === mid)?.id ?? null;
  }

  /** Whether the seat's camera is taken as on (`showing`); on, its clock runs from now if it did not. */
  setShowing(seat: number, on: boolean) {
    if (!on) this.pictureAt.delete(seat);
    else if (!this.pictureAt.has(seat)) this.pictureAt.set(seat, Date.now());
    if ((this.showing[seat] ?? false) === on) return;
    this.showing = { ...this.showing, [seat]: on };
    this.refocus();
  }

  /**
   * The seats whose pictures have been missing for `VIDEO_LOST_MS` have
   * their camera off: looked at by the clock of the room, once a second,
   * whether a tile of theirs is there or not.
   */
  private lapse() {
    const now = Date.now();
    for (const [seat, at] of [...this.pictureAt]) {
      if (now - at >= VIDEO_LOST_MS) this.setShowing(seat, false);
    }
    for (const [seat, at] of this.lastPicture) {
      if (now - at >= PICTURE_FLOWING_MS) this.flag('flowing', seat, false);
    }
    for (const [seat, since] of this.toldAt) {
      this.flag('stale', seat, now - Math.max(since, this.lastPicture.get(seat) ?? 0) >= VIDEO_LOST_MS);
    }
    // A carried word whose seat said none of its own in time: the seat is
    // judged by its frames from now on.
    let lapsed = false;
    for (const [npub, c] of [...this.carried]) {
      if (c.until != null && now >= c.until) {
        this.carried.delete(npub);
        lapsed = true;
      }
    }
    if (lapsed && this.raw) this.setCall(this.raw);
  }

  /**
   * The large place by the voices, again: what `focusOf` names now, held
   * (`holdFocus`); a change held back is looked at again once its hold is over.
   */
  private refocus() {
    if (this.voiceTimer) clearTimeout(this.voiceTimer);
    this.voiceTimer = null;
    const c = this.call;
    if (!c) {
      this.voice = null;
      this.voiceSince = 0;
      return;
    }
    const showing = (seat: number) => !!this.showing[seat];
    const fits = (seat: number) => c.participants.some((p) => p.id === seat && !p.me && seatSendsVideo(p, c, showing));
    const candidate = focusOf(c.participants, null, this.speaker, showing);
    const now = Date.now();
    const next = holdFocus({ seat: this.voice, since: this.voiceSince }, candidate, fits, now);
    if (this.voice !== next.seat) this.voice = next.seat;
    this.voiceSince = next.since;
    if (candidate != null && candidate !== next.seat) {
      this.voiceTimer = setTimeout(() => this.refocus(), Math.max(0, next.since + FOCUS_HOLD_MS - now));
    }
  }

  /** Runtime event → state. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    const p = (ev.payload ?? {}) as Record<string, unknown>;
    switch (ev.name) {
      case 'group_call.state': {
        const view = p.call as GroupCallView | undefined;
        if (!view?.call_id) break;
        if (view.phase === 'left') {
          if (this.call?.call_id === view.call_id) this.finish(view, false);
          break;
        }
        // A late word of a room that is over is not a new room.
        if (this.isFinished(view.call_id) && this.call?.call_id !== view.call_id) break;
        this.setCall(view);
        break;
      }
      case 'group_call.started': {
        const a = p.call as GroupCallAnnounced | undefined;
        if (!a?.group_id) break;
        this.announcedEpoch++;
        this.announced = { ...this.announced, [a.group_id]: a };
        break;
      }
      case 'group_call.ended': {
        const a = p.call as GroupCallAnnounced | undefined;
        if (!a?.group_id) break;
        if (this.announced[a.group_id]?.call_id === a.call_id) this.dropAnnounced(a.group_id);
        // The room that just went here: over for everybody, not only left by me.
        const o = this.over;
        if (o && o.call.call_id === a.call_id && o.how !== 'left') this.over = { ...o, how: p.outcome === 'failed' ? 'failed' : 'ended' };
        break;
      }
      case 'group_call.level': {
        const seat = p.participant;
        if (!this.call || p.call_id !== this.call.call_id || typeof seat !== 'number' || typeof p.level !== 'number') break;
        const level = Math.max(0, Math.min(1, p.level));
        if (this.levels[seat] !== level) this.levels = { ...this.levels, [seat]: level };
        break;
      }
      case 'error': {
        if (p.scope !== 'group_calls') break;
        // A word of the room, not the answer of a command (a camera that is
        // not there, a way restored): shown for a while, then let go.
        this.tell(p.error ?? null);
        break;
      }
      // A phone: my camera would not open for the room; my video is off
      // again in the core by then (the shell turned it off), the room
      // goes on without it.
      case 'call.camera_failed':
        if (this.call && p.callId === this.call.call_id) this.tell(p.error ?? null);
        break;
      // A phone: Hang up pressed on the notification of the room. The
      // shell leaves the room by itself, past `leave`: the end is mine.
      case 'call.debug':
        if (p.action === 'hangup' && this.call && p.callId === this.call.call_id) this.leaving = this.call.call_id;
        break;
      // A headset came or went, or the phone moved the sound by itself.
      case 'call.audio_route':
        if (Array.isArray(p.available)) this.routes = { current: (p.current as CallAudioRoute | null) ?? null, available: p.available as CallAudioRoute[] };
        break;
    }
  }

  private dropAnnounced(groupId: string) {
    this.announcedEpoch++;
    const { [groupId]: _gone, ...rest } = this.announced;
    this.announced = rest;
  }

  private setCall(view: GroupCallView | null) {
    const last = this.call;
    const before = last?.call_id;
    if (!view || view.call_id !== before) {
      this.words.clear();
      this.carried.clear();
    }
    this.raw = view;
    if (view) view = this.remember(view, last);
    this.epoch++;
    this.call = view;
    if (view) {
      const home = homeOf(view);
      // Another room of the same call: the runtime says `joining` with no
      // seat (not even mine) once it leaves the room I sat in for the one
      // the room moved to, also on the node of the old one.
      const newRoom = !!last && view.call_id === before && view.phase === 'joining' && view.participants.length === 0 && last.participants.length > 0;
      if (view.call_id !== before) {
        this.settledHome = null;
        this.anotherRoom = false;
        this.levels = {};
        this.forgetSeats();
        this.speaker = null;
        this.error = null;
        this.screen = false;
        this.shown = true;
        this.cameraBeforeScreen = false;
        this.layers.clear();
        this.simulcast = true;
        this.routes = null;
        this.dropOver();
      } else if (last && (homeOf(last) !== home || newRoom)) {
        // The same call in another room (it moved, mostly to another node):
        // its seats are numbered anew, what was asked of the old ones is
        // asked again. The screen, my camera and my screen stay.
        if (newRoom) this.anotherRoom = true;
        this.levels = {};
        this.forgetSeats();
        this.speaker = null;
        this.layers.clear();
        this.simulcast = true;
      }
      if (view.phase === 'in_room') {
        this.settledHome = home;
        this.anotherRoom = false;
      }
      this.moving = view.phase !== 'in_room' && this.settledHome != null && (home !== this.settledHome || this.anotherRoom);
      const talking = view.participants.find((p) => p.speaking && !p.me && p.verified);
      if (talking) this.speaker = talking.id;
      else if (this.speaker != null && !view.participants.some((p) => p.id === this.speaker)) this.speaker = null;
      // A seat that left, or whose video has no m-line now, has no camera on.
      const kept = Object.entries(this.showing).filter(([seat]) => view.participants.some((p) => p.id === Number(seat) && p.video_mid));
      if (kept.length !== Object.keys(this.showing).length) this.showing = Object.fromEntries(kept);
      this.followWords(view, last);
    } else {
      this.settledHome = null;
      this.anotherRoom = false;
      this.moving = false;
      this.levels = {};
      this.forgetSeats();
      this.speaker = null;
      this.screen = false;
      this.cameraBeforeScreen = false;
      this.layers.clear();
      this.routes = null;
    }
    // The clock of a seat let go goes with it.
    for (const seat of [...this.pictureAt.keys()]) if (!this.showing[seat]) this.pictureAt.delete(seat);
    this.refocus();
    this.tick();
  }

  /** The clocks and flags of the seats let go: another room, or the seats numbered anew. */
  private forgetSeats() {
    this.showing = {};
    this.flowing = {};
    this.stale = {};
    this.lastPicture.clear();
    this.toldAt.clear();
  }

  /**
   * The words of state kept (`words`) and carried over a rejoin
   * (`carried`): a confirmed seat that says none now is given the one its
   * person said before the rejoin, while that person has this one seat and
   * the carried word has not lapsed; the view as it came when none is.
   * Outside a rejoin a seat that says nothing is judged by its frames,
   * whatever another seat of the same person says.
   */
  private remember(view: GroupCallView, last: GroupCallView | null): GroupCallView {
    // A rejoin (the core leaves the room for a new session of it: the way
    // restored, the room moved): the seats come anew, their words after
    // their words of identity. What the people said goes over with them.
    if (last?.call_id === view.call_id && last.phase !== 'joining' && view.phase === 'joining') {
      for (const [npub, word] of this.words) this.carried.set(npub, { word, until: null });
    }
    const seats = new Map<string, number>();
    for (const p of view.participants) if (!p.me && p.verified && p.npub) seats.set(p.npub, (seats.get(p.npub) ?? 0) + 1);
    const now = Date.now();
    let filled = false;
    const participants = view.participants.map((p) => {
      if (p.me || !p.verified || !p.npub) return p;
      const one = seats.get(p.npub) === 1;
      if (p.camera != null || p.mic != null || p.screen != null) {
        if (one) this.words.set(p.npub, { camera: p.camera, mic: p.mic, screen: p.screen });
        else this.words.delete(p.npub);
        this.carried.delete(p.npub);
        return p;
      }
      // Two seats of one person: a word does not tell which device said it.
      if (!one) {
        this.words.delete(p.npub);
        this.carried.delete(p.npub);
        return p;
      }
      const c = this.carried.get(p.npub);
      if (!c) return p;
      if (c.until == null) c.until = now + CARRIED_WORD_MS;
      else if (now >= c.until) {
        this.carried.delete(p.npub);
        return p;
      }
      filled = true;
      return { ...p, ...c.word };
    });
    // A person who left: every seat is confirmed and none is theirs. (A
    // seat not confirmed yet may be theirs come again.)
    if (view.phase === 'in_room' && view.participants.every((p) => p.verified)) {
      const here = new Set(view.participants.map((p) => p.npub));
      for (const npub of [...this.words.keys()]) if (!here.has(npub)) this.words.delete(npub);
      for (const npub of [...this.carried.keys()]) if (!here.has(npub)) this.carried.delete(npub);
    }
    return filled ? { ...view, participants } : view;
  }

  /**
   * The clock of `stale` follows each seat's word: it runs from when the
   * word says the seat's video is on, and stops when it says off. A camera
   * on again after its word said off waits for its pictures anew: none is
   * on its way (a tile draws nothing while the word says off), and a tile
   * made anew shows the face, not a dark box (`flowing`).
   */
  private followWords(view: GroupCallView, last: GroupCallView | null) {
    const before = new Map(last?.call_id === view.call_id ? last.participants.map((p) => [p.id, p]) : []);
    const now = Date.now();
    for (const seat of [...this.lastPicture.keys()]) {
      if (view.participants.some((p) => p.id === seat && p.video_mid)) continue;
      this.lastPicture.delete(seat);
      this.flag('flowing', seat, false);
    }
    for (const seat of [...this.toldAt.keys()]) if (!view.participants.some((p) => p.id === seat)) this.toldAt.delete(seat);
    for (const p of view.participants) {
      if (p.me) continue;
      if (seatVideoWord(p) !== true) {
        this.toldAt.delete(p.id);
        continue;
      }
      if (this.toldAt.has(p.id)) continue;
      this.toldAt.set(p.id, now);
      if (before.get(p.id) && seatVideoWord(before.get(p.id)!) === false) {
        this.lastPicture.delete(p.id);
        this.flag('flowing', p.id, false);
      }
    }
    for (const seat of Object.keys(this.stale).map(Number)) if (!this.toldAt.has(seat)) this.flag('stale', seat, false);
  }

  /**
   * The room is over here. `mine`: said by this page after its own Leave,
   * before the runtime's word; the runtime's word changes nothing then.
   */
  private finish(view: GroupCallView, mine: boolean) {
    if (this.over?.call.call_id === view.call_id) return;
    const last = this.call?.call_id === view.call_id ? this.call : view;
    if (this.call?.call_id === view.call_id) this.setCall(null);
    this.epoch++;
    if (!this.isFinished(view.call_id)) this.finished = [...this.finished.slice(-15), { callId: view.call_id, groupId: view.group_id }];
    const left = mine || this.leaving === view.call_id;
    if (this.leaving === view.call_id) this.leaving = null;
    const wasIn = last.phase === 'in_room' || last.phase === 'reconnecting';
    // A start or a join that failed before I was in the room: the button
    // that asked shows the refusal; no room screen says it again.
    if (!left && !wasIn && this.entering > 0) {
      this.dropOver();
      return;
    }
    // Never in the room: it did not come about; out of it without my word: it went.
    const how: GroupOver = left ? 'left' : this.error || !wasIn ? 'failed' : 'ended';
    this.over = { call: { ...last, phase: 'left' }, how };
    if (this.overTimer) clearTimeout(this.overTimer);
    this.overTimer = setTimeout(() => this.dropOver(), GROUP_OVER_SHOWN_MS);
  }

  private isFinished(callId: string): boolean {
    return this.finished.some((f) => f.callId === callId);
  }

  private dropOver() {
    if (this.overTimer) clearTimeout(this.overTimer);
    this.overTimer = null;
    this.over = null;
  }

  /** The clock runs while I am in a room; it lets go the cameras whose pictures stopped (`lapse`). */
  private tick() {
    this.now = Math.floor(Date.now() / 1000);
    if (this.call && !this.ticker) {
      this.ticker = setInterval(() => {
        this.now = Math.floor(Date.now() / 1000);
        this.lapse();
      }, 1000);
    } else if (!this.call && this.ticker) {
      clearInterval(this.ticker);
      this.ticker = null;
    }
  }

  reset() {
    this.setCall(null);
    this.dropOver();
    this.announced = {};
    this.busy = false;
    this.ending = false;
    this.error = null;
    this.finished = [];
    this.entering = 0;
    this.leaving = null;
    this.shown = true;
  }
}

export const groupCallStore = new GroupCallStore();

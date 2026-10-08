// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The call under way and the settings of calls. Fed by the runtime's
// `call.*` events, which the module store forwards here; the screens of a
// call (the desk's panel and incoming card, the phone's call page) read it.
//
// Only one call is ever under way: the runtime answers a second one with
// "busy". Whatever the latest event says the call is, it is; a call that
// ended stays as `ended` for a moment, so the screen can say how.

import {
  messengerApi,
  type CallAudioRoute,
  type CallAudioRoutes,
  type CallMedia,
  type CallNodeView,
  type CallOutcome,
  type CallStats,
  type CallView,
  type CameraInfo,
  type MessengerUiEvent,
  type RelayPolicy,
  type ScreenInfo,
  type VideoInput,
  type VideoQuality,
} from '../api';

/** How long the screen shows how a call ended. */
export const ENDED_SHOWN_MS = 2500;

export interface EndedCall {
  call: CallView;
  outcome: CallOutcome;
  /** Seconds it was answered; `null` for one that never was. */
  duration: number | null;
  /**
   * Ended by this device's own hang-up or refusal (not by the peer, a
   * timeout or a failure), from this page or from outside it (the call's
   * notification, a headset button).
   */
  local: boolean;
}

/** How long my call rings before the runtime gives it up as not answered (`RING_TIMEOUT` of the calls crate, the invite's 45 s). */
export const RING_TIMEOUT_SECS = 45;

/**
 * My call that nobody answered, ended as "missed" before its ring ran out:
 * I gave it up myself. The runtime's `call.ended` does not say who ended a
 * call, and a hang-up from the notification or a headset never passes this
 * page; but an unanswered call of mine is "missed" only by my own end or by
 * the ring's timeout (a peer declines or is busy, a line fails), and the
 * timeout comes no sooner than `RING_TIMEOUT_SECS` after the call began.
 */
export function gaveUp(view: CallView, outcome: CallOutcome, now: number): boolean {
  return view.direction === 'out' && view.answered_at == null && outcome === 'missed' && now - view.started_at < RING_TIMEOUT_SECS;
}

/** Where the choice "ask before a call" is kept: this device only, like the place of the call's window. */
export const CONFIRM_KEY = 'messenger.calls.confirm';

/** How far a call has come; `reconnecting` is as far as `active` (a talk whose way is being restored), so that the two go back and forth. */
const PHASE_ORDER: Record<CallView['phase'], number> = { incoming: 0, outgoing: 0, connecting: 1, active: 2, reconnecting: 2, ended: 3 };

class CallStore {
  /** The call under way; `null` when there is none. */
  call = $state<CallView | null>(null);
  /** The call that just ended, for a moment. */
  ended = $state<EndedCall | null>(null);
  /** What the engine last told of the call under way. */
  stats = $state<CallStats | null>(null);
  /** How loud the peer is, 0..1. */
  level = $state(0);
  /** The clock of the call under way, unix seconds, ticking while it talks. */
  now = $state(Math.floor(Date.now() / 1000));
  policy = $state<RelayPolicy>('auto');
  /** Every node a call may use, the most preferred first. */
  nodes = $state<CallNodeView[]>([]);
  /** This build has a media engine: calls can be made and answered. */
  available = $state(false);
  loaded = $state(false);
  /** A command of the call is on its way. */
  busy = $state(false);
  /**
   * A hang-up or a refusal is on its way. Kept apart from `busy`: End and
   * Decline must work while another command (a start that still picks its
   * nodes, an answer, a camera) has not come back.
   */
  ending = $state(false);
  /** The last refusal or failure, as the runtime said it; the screen words it. */
  error = $state<unknown>(null);
  /**
   * A phone: where the sound goes and where it could; `null` where the
   * host has no say in it (a computer) or before it was asked.
   */
  routes = $state<CallAudioRoutes | null>(null);
  /** How big my video goes, from the next time it goes on. */
  videoQuality = $state<VideoQuality>('360p');
  /**
   * Calls ring on this device. `null`: the runtime does not say (a build
   * without the setting), and the settings do not offer it.
   */
  incoming = $state<boolean | null>(null);
  /** A call button asks "voice call?" / "video call?" first, so that none is made by accident. Kept on this device. */
  confirm = $state(readConfirm());
  /** A computer's cameras, as last asked; empty on a phone. */
  cameras = $state<CameraInfo[]>([]);
  /** The screens and windows that can be shown, as last asked. */
  screens = $state<ScreenInfo[]>([]);

  /** My camera was on when my screen went on: stopping the screen brings it back. */
  private cameraBeforeScreen = false;

  /** The call this page is hanging up or refusing: its end is mine, whichever word of it comes first. */
  private leaving: string | null = null;

  private endedTimer: ReturnType<typeof setTimeout> | null = null;
  /**
   * Counts every change of the call in this store. A snapshot or an answer
   * asked before a change and come after it is older than what the store
   * holds: an event and the answer of a command travel apart.
   */
  private epoch = 0;
  /** The calls that ended here lately, kept longer than `ended` is shown: none of them comes back. */
  private over: string[] = [];
  private ticker: ReturnType<typeof setInterval> | null = null;

  /** Own nodes (the developer setting), in their order. */
  get ownNodes(): CallNodeView[] {
    return this.nodes.filter((n) => n.class === 'own');
  }

  /** A call rings here, waiting for an answer. */
  get ringing(): boolean {
    return this.call?.phase === 'incoming';
  }

  /** The call under way has video to show: mine, the peer's, or both (not while it rings here). */
  get video(): boolean {
    const c = this.call;
    return !!c && c.phase !== 'incoming' && (c.video_local || c.video_remote);
  }

  /** A call can be made now: this build has the engine and none is under way. */
  canCall(): boolean {
    return this.available && !this.call;
  }

  async load() {
    const at = this.epoch;
    const s = await messengerApi.calls.state();
    this.policy = s.policy;
    this.nodes = s.nodes;
    this.available = s.available;
    this.videoQuality = s.video_quality ?? '360p';
    this.incoming = typeof s.incoming_enabled === 'boolean' ? s.incoming_enabled : null;
    this.loaded = true;
    // The call changed here while the snapshot was on its way: the events
    // are the runtime's word as it changes, and any change after them comes
    // as an event too, so the snapshot is the older one.
    if (this.epoch !== at) return;
    // A call taken before the page was there (the phone's Answer started the app).
    const live = s.call && s.call.phase !== 'ended' && !this.isOver(s.call.call_id) ? s.call : null;
    if (live) this.setCall(live);
    else if (this.call) this.setCall(null);
  }

  async start(peer: string, media: CallMedia = 'audio') {
    const at = this.epoch;
    return this.run(async () => {
      try {
        return this.adopt(await messengerApi.calls.start(peer, media));
      } catch (e) {
        // The call showed and ended while the command was on its way (given
        // up while its nodes were picked, or it failed): the screen already
        // says how, a refusal on top would say it twice.
        if (this.epoch !== at && !this.call && this.ended) return undefined;
        throw e;
      }
    });
  }

  /**
   * Starts a call; a refusal is handed back rather than kept, for the
   * button that shows it beside itself: one kept in the store would show
   * later on a screen it has nothing to do with.
   */
  async dial(peer: string, media: CallMedia = 'audio'): Promise<{ view: CallView | null; refusal: unknown }> {
    const view = await this.start(peer, media);
    if (view) return { view, refusal: null };
    const refusal = this.error;
    this.error = null;
    return { view: null, refusal };
  }

  async accept() {
    const c = this.call;
    if (!c) return;
    const view = await this.run(async () => this.adopt(await messengerApi.calls.accept(c.call_id)));
    if (view === undefined) await this.resync();
  }

  async decline() {
    const c = this.call;
    if (!c) return;
    this.leaving = c.call_id;
    await this.ender(async () => {
      await messengerApi.calls.decline(c.call_id);
      // The runtime says `call.ended` too; the screen goes at once.
      this.finish(c, 'declined', null, true);
    });
  }

  /** Hangs up, or gives up calling. */
  async hangUp() {
    const c = this.call;
    if (!c) return;
    this.leaving = c.call_id;
    await this.ender(async () => {
      await messengerApi.calls.end(c.call_id);
      const duration = c.answered_at ? Math.max(0, Math.floor(Date.now() / 1000) - c.answered_at) : null;
      this.finish(c, c.answered_at ? 'ended' : 'missed', duration, true);
    });
  }

  /**
   * The answer of a start or an accept, unless the store knows better: the
   * call ended meanwhile (its `call.ended` came first), or an event already
   * took it further (`active` before the answer's `connecting`).
   */
  private adopt(view: CallView): CallView {
    if (this.isOver(view.call_id)) return view;
    const c = this.call;
    if (c?.call_id === view.call_id && PHASE_ORDER[c.phase] > PHASE_ORDER[view.phase]) return view;
    this.setCall(view);
    return view;
  }

  /**
   * A command about the call failed: the runtime may no longer have the
   * call the store shows (it ended unseen). The state is asked again, and
   * a refusal about a call that is gone goes with it.
   */
  private async resync() {
    await this.load().catch(() => {});
    if (!this.call) this.error = null;
  }

  /**
   * Takes a ringing video call with my camera off: the runtime turns the
   * camera of a video call on as it is taken, so it is turned off at once.
   */
  async acceptWithoutVideo() {
    await this.accept();
    if (this.call && this.call.phase !== 'incoming' && !this.error) await this.applyVideo({ kind: 'off' });
  }

  /** My camera on, or off; a screen being shown gives way to the camera. */
  async toggleCamera() {
    const c = this.call;
    if (!c) return;
    this.cameraBeforeScreen = false;
    await this.applyVideo(c.video_local && !c.video_screen ? { kind: 'off' } : { kind: 'camera', id: c.camera });
  }

  /** The next camera: the other side of a phone, the next one listed on a computer. */
  async switchCamera() {
    await this.run(async () => this.take(await messengerApi.calls.switchCamera()));
  }

  /** My screen (or the window `id`) instead of my camera. */
  async shareScreen(id?: string) {
    const c = this.call;
    if (!c) return;
    const camera = c.video_local && !c.video_screen;
    const view = await this.run(async () => this.take(await messengerApi.calls.shareScreen(id)));
    if (view) this.cameraBeforeScreen = camera;
  }

  /** The screen no more: back to the camera when it was on before, off otherwise. */
  async stopScreen() {
    const c = this.call;
    if (!c) return;
    const back = this.cameraBeforeScreen;
    this.cameraBeforeScreen = false;
    await this.applyVideo(back ? { kind: 'camera', id: c.camera } : { kind: 'off' });
  }

  async loadCameras() {
    try {
      this.cameras = await messengerApi.calls.cameras();
    } catch {
      this.cameras = [];
    }
  }

  /** Asked each time the picker opens: windows come and go. `null` when the asking failed (the error is kept). */
  async loadScreens(): Promise<ScreenInfo[] | null> {
    this.error = null;
    try {
      this.screens = await messengerApi.calls.screens();
      return this.screens;
    } catch (e) {
      this.screens = [];
      this.error = e;
      return null;
    }
  }

  /** Calls ring on this device, or do not (the other devices of mine still ring). */
  async setIncoming(enabled: boolean) {
    await this.run(async () => {
      const s = await messengerApi.calls.setIncoming(enabled);
      this.incoming = typeof s.incoming_enabled === 'boolean' ? s.incoming_enabled : enabled;
    });
  }

  /** Ask before a call, or not; kept on this device. */
  setConfirm(on: boolean) {
    this.confirm = on;
    try { localStorage.setItem(CONFIRM_KEY, on ? '1' : '0'); } catch { /* kept until the page closes */ }
  }

  async setVideoQuality(quality: VideoQuality) {
    await this.run(async () => {
      const s = await messengerApi.calls.setVideoQuality(quality);
      this.videoQuality = s.video_quality;
    });
  }

  private async applyVideo(input: VideoInput) {
    await this.run(async () => this.take(await messengerApi.calls.setVideo(input)));
  }

  /** The answer of a command about the call under way, unless another call took its place. */
  private take(view: CallView): CallView {
    if (this.call?.call_id === view.call_id) this.setCall(view);
    return view;
  }

  async toggleMute() {
    const c = this.call;
    if (!c) return;
    await this.run(async () => {
      const view = await messengerApi.calls.mute(!c.muted);
      if (this.call?.call_id === view.call_id) this.setCall(view);
    });
  }

  async setPolicy(policy: RelayPolicy) {
    await this.run(async () => {
      const s = await messengerApi.calls.setPolicy(policy);
      this.policy = s.policy;
      this.nodes = s.nodes;
    });
  }

  /**
   * Adds an own node in front of the others. The runtime refuses what is
   * not `address:port#id`. Nodes already listed keep their access keys
   * (sent without one, a reference keeps the key it has).
   */
  async addNode(reference: string, key?: string) {
    const ref = reference.trim();
    const others = this.ownNodes.filter((n) => n.reference !== ref).map((n) => ({ reference: n.reference }));
    return this.saveNodes([{ reference: ref, key: key?.trim() || undefined }, ...others]);
  }

  async removeNode(reference: string) {
    return this.saveNodes(this.ownNodes.filter((n) => n.reference !== reference).map((n) => ({ reference: n.reference })));
  }

  private async saveNodes(list: { reference: string; key?: string }[]): Promise<boolean> {
    const done = await this.run(async () => {
      const s = await messengerApi.calls.setNodes(list);
      this.nodes = s.nodes;
      return true;
    });
    return done === true;
  }

  /** A phone's call screen asks once the sound is the call's; a computer refuses, and nothing is shown. */
  async loadRoutes() {
    try {
      this.routes = await messengerApi.calls.audioRoute('list');
    } catch {
      this.routes = null;
    }
  }

  /**
   * The loudspeaker on, or off again: back to a headset when one is there,
   * the earpiece otherwise.
   */
  async toggleSpeaker() {
    const r = this.routes;
    if (!r) return;
    const order: CallAudioRoute[] = ['bluetooth', 'wired', 'earpiece'];
    const next = r.current === 'speaker' ? (order.find((x) => r.available.includes(x)) ?? 'earpiece') : 'speaker';
    await this.run(async () => { this.routes = await messengerApi.calls.audioRoute('set', next); });
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

  /** A hang-up or a refusal: as `run`, on `ending`; when it fails, the runtime is asked what is left. */
  private async ender(fn: () => Promise<void>) {
    this.ending = true;
    this.error = null;
    let failed = false;
    try {
      await fn();
    } catch (e) {
      this.error = e;
      failed = true;
      this.leaving = null;
    } finally {
      this.ending = false;
    }
    if (failed) await this.resync();
  }

  clearError() {
    this.error = null;
  }

  /** Runtime event → state. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    const p = (ev.payload ?? {}) as Record<string, unknown>;
    switch (ev.name) {
      case 'call.incoming':
      case 'call.state': {
        const view = p.call as CallView | undefined;
        if (!view || view.phase === 'ended') break;
        // A late word of a call that is over is not a new call.
        if (this.isOver(view.call_id) && this.call?.call_id !== view.call_id) break;
        this.setCall(view);
        break;
      }
      case 'call.ended': {
        const view = p.call as CallView | undefined;
        if (!view) break;
        const outcome = (p.outcome as CallOutcome | undefined) ?? 'ended';
        const duration = typeof p.duration_secs === 'number' ? p.duration_secs : null;
        // A call missed while I was away has no screen to close.
        if (this.call?.call_id === view.call_id || this.ended?.call.call_id === view.call_id) this.finish(view, outcome, duration, false);
        break;
      }
      case 'call.stats':
        if (this.call && p.call_id === this.call.call_id) this.stats = (p.stats as CallStats | undefined) ?? null;
        break;
      case 'call.level':
        if (this.call && p.call_id === this.call.call_id && typeof p.level === 'number') this.level = Math.max(0, Math.min(1, p.level));
        break;
      case 'error':
        if (p.scope === 'calls') this.error = p.error ?? null;
        break;
      // A headset came or went, or the phone moved the sound by itself.
      case 'call.audio_route':
        if (Array.isArray(p.available)) this.routes = { current: (p.current as CallAudioRoute | null) ?? null, available: p.available as CallAudioRoute[] };
        break;
    }
  }

  private setCall(view: CallView | null) {
    const before = this.call?.call_id;
    this.epoch++;
    this.call = view;
    if (view) {
      if (view.call_id !== before) {
        this.stats = null;
        this.level = 0;
        this.error = null;
        this.dropEnded();
      }
      this.tick();
    } else {
      this.stats = null;
      this.level = 0;
      this.routes = null;
      this.cameraBeforeScreen = false;
      this.tick();
    }
  }

  /**
   * The call is over. `guess`: said by this page after its own command,
   * before the runtime's `call.ended`; the runtime's word replaces it.
   */
  private finish(view: CallView, outcome: CallOutcome, duration: number | null, guess: boolean) {
    const known = this.ended?.call.call_id === view.call_id;
    if (known && guess) return;
    if (this.call?.call_id === view.call_id) this.setCall(null);
    this.epoch++;
    if (!this.isOver(view.call_id)) this.over = [...this.over.slice(-15), view.call_id];
    const local = guess || this.leaving === view.call_id || gaveUp(view, outcome, Date.now() / 1000);
    if (this.leaving === view.call_id) this.leaving = null;
    this.ended = { call: { ...view, phase: 'ended' }, outcome, duration, local };
    if (this.endedTimer) clearTimeout(this.endedTimer);
    this.endedTimer = setTimeout(() => this.dropEnded(), ENDED_SHOWN_MS);
  }

  private isOver(callId: string): boolean {
    return this.over.includes(callId);
  }

  private dropEnded() {
    if (this.endedTimer) clearTimeout(this.endedTimer);
    this.endedTimer = null;
    this.ended = null;
  }

  /** The clock runs while a call is under way. */
  private tick() {
    this.now = Math.floor(Date.now() / 1000);
    if (this.call && !this.ticker) {
      this.ticker = setInterval(() => (this.now = Math.floor(Date.now() / 1000)), 1000);
    } else if (!this.call && this.ticker) {
      clearInterval(this.ticker);
      this.ticker = null;
    }
  }

  reset() {
    this.setCall(null);
    this.dropEnded();
    this.nodes = [];
    this.policy = 'auto';
    this.available = false;
    this.loaded = false;
    this.busy = false;
    this.ending = false;
    this.error = null;
    this.over = [];
    this.incoming = null;
    this.leaving = null;
  }
}

/** "Ask before a call" as kept on this device; on unless turned off. */
function readConfirm(): boolean {
  try {
    return localStorage.getItem(CONFIRM_KEY) !== '0';
  } catch {
    return true;
  }
}

export const callStore = new CallStore();

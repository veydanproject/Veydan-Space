<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Recording under a held button, in place of the field of the phone composer:
  a voice message or a circle. Recording starts as soon as this is shown; the
  composer owns the button and tells this what the finger did. A recording
  that was let go is played back here before it is sent.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { CANCEL_PX, LOCK_PX } from '../dm/record-gesture';
  import { NO_POSTER, showFirstFrame } from './blob-url';
  import { CaptureError, LIMIT_SECS, Recorder, clockOf, closeStream, openStream, type CaptureKind, type Captured } from './capture';

  interface Props {
    kind: CaptureKind;
    /** The finger left and the recording goes on. */
    locked: boolean;
    /** How far the held button was dragged, pixels (never positive). */
    dx?: number;
    dy?: number;
    phase?: 'recording' | 'preview';
    oncancel: () => void;
    ondone: (c: Captured) => void;
    onerror: (code: string) => void;
  }
  let { kind, locked, dx = 0, dy = 0, phase = $bindable('recording'), oncancel, ondone, onerror }: Props = $props();

  /** Shorter than a breath is a mis-tap, not a message. */
  const MIN_MS: Record<CaptureKind, number> = { voice: 600, circle: 800 };
  const FACING_KEY = 'veydan.msg.rec.facing';
  const R = 148;
  const C = 2 * Math.PI * R;

  let live = $state<HTMLVideoElement | null>(null);
  let playback = $state<HTMLVideoElement | null>(null);
  let stream: MediaStream | null = null;
  let recorder: Recorder | null = null;
  let audio: HTMLAudioElement | null = null;
  let timer: ReturnType<typeof setInterval> | null = null;
  let gone = false;
  let finishing = false;

  let ready = $state(false);
  let elapsed = $state(0);
  let bars = $state<number[]>([]);
  let facing = $state<'user' | 'environment'>(savedFacing());
  let captured = $state<Captured | null>(null);
  let url = $state('');
  let playing = $state(false);
  let played = $state(0);

  const progress = $derived(Math.min(1, elapsed / (LIMIT_SECS.circle * 1000)));

  function savedFacing(): 'user' | 'environment' {
    try { return localStorage.getItem(FACING_KEY) === 'environment' ? 'environment' : 'user'; } catch { return 'user'; }
  }

  function release() {
    if (timer) clearInterval(timer);
    timer = null;
    closeStream(stream);
    stream = null;
  }

  async function open() {
    try {
      const s = await openStream(kind, facing);
      if (gone) { closeStream(s); return; }
      stream = s;
      if (live) { live.srcObject = s; live.play().catch(() => {}); }
      recorder = new Recorder(s, kind, (l) => { bars = [...bars.slice(-39), l]; });
      recorder.start();
      ready = true;
      timer = setInterval(() => {
        elapsed = recorder?.elapsedMs() ?? 0;
        if (elapsed >= LIMIT_SECS[kind] * 1000) stop();
      }, 100);
    } catch (e) {
      if (!gone) onerror(e instanceof CaptureError ? e.code : 'unsupported');
    }
  }

  async function take(): Promise<Captured | null> {
    if (!recorder || finishing) return null;
    finishing = true;
    try {
      const c = await recorder.stop();
      release();
      recorder = null;
      if (c.durationMs < MIN_MS[kind] || c.blob.size === 0) { oncancel(); return null; }
      return c;
    } catch {
      release();
      onerror('unsupported');
      return null;
    } finally {
      finishing = false;
    }
  }

  /** End the recording and show it before it is sent. */
  export async function stop() {
    if (phase === 'preview' || finishing) return;
    // Let go before the microphone answered: there is nothing to keep.
    if (!recorder) { cancel(); return; }
    const c = await take();
    if (!c || gone) return;
    captured = c;
    url = URL.createObjectURL(c.blob);
    phase = 'preview';
  }

  export function cancel() {
    recorder?.cancel();
    recorder = null;
    release();
    pause();
    oncancel();
  }

  /** Send what was recorded: the one under review, or the one still going. */
  export async function send() {
    if (phase === 'preview') {
      pause();
      if (captured) ondone(captured);
      return;
    }
    const c = await take();
    if (c) ondone(c);
  }

  // The other camera starts the take again: one recording has one camera.
  async function flip() {
    if (finishing || phase === 'preview') return;
    recorder?.cancel();
    recorder = null;
    release();
    ready = false;
    elapsed = 0;
    bars = [];
    facing = facing === 'user' ? 'environment' : 'user';
    try { localStorage.setItem(FACING_KEY, facing); } catch { /* the choice is not kept */ }
    await open();
  }

  function player(): HTMLMediaElement | null {
    if (kind === 'circle') return playback;
    if (!audio && url) audio = new Audio(url);
    return audio;
  }

  /** The user asked for the sound; the silent first frame is not that. */
  let wanted = false;

  function pause() {
    wanted = false;
    audio?.pause();
    playback?.pause();
    playing = false;
  }

  function toggle() {
    const m = player();
    if (!m || !captured) return;
    if (playing) { pause(); return; }
    const total = captured.durationMs;
    // A fresh recording has no length of its own: ours is the measure.
    m.ontimeupdate = () => { played = Math.min(1, (m.currentTime * 1000) / total); };
    m.onended = () => { playing = false; played = 0; wanted = false; };
    wanted = true;
    m.muted = false;
    m.play().then(() => { playing = true; }).catch(() => {});
  }

  const tenths = (ms: number) => {
    const s = Math.floor(ms / 1000);
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')},${Math.floor(ms / 100) % 10}`;
  };

  onMount(() => {
    open();
    return () => {
      gone = true;
      recorder?.cancel();
      release();
      audio?.pause();
      if (url) URL.revokeObjectURL(url);
    };
  });

  const keep = (e: Event) => e.preventDefault();
</script>

{#if kind === 'circle'}
  <div class="stage">
    <div class="finder">
      <svg class="ring" viewBox="0 0 304 304" aria-hidden="true">
        <circle cx="152" cy="152" r={R} class="track" />
        <circle cx="152" cy="152" r={R} class="fill" stroke-dasharray={C} stroke-dashoffset={C * (1 - (phase === 'preview' ? played : progress))} />
      </svg>
      {#if phase === 'preview'}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video bind:this={playback} src={url} poster={NO_POSTER} preload="auto" playsinline onloadeddata={() => { if (playback) showFirstFrame(playback, () => wanted); }}></video>
        <button class="over" onpointerdown={keep} onclick={toggle} aria-label={playing ? $t('msg_media_pause') : $t('msg_voice_play')}>
          {#if !playing}<Icon name="play" size={40} />{/if}
        </button>
      {:else}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video bind:this={live} class:mirror={facing === 'user'} muted playsinline></video>
      {/if}
    </div>
    {#if locked && phase === 'recording'}
      <button class="flip" disabled={!ready} onpointerdown={keep} onclick={flip} aria-label={$t('msg_rec_flip')}><Icon name="switch-camera" size={20} /></button>
    {/if}
  </div>
{/if}

{#if phase === 'recording' && !locked}
  <div class="lock" style:transform="translateY({dy * 0.25}px) scale({1 + Math.min(1, -dy / LOCK_PX) * 0.18})" aria-hidden="true">
    <Icon name="lock" size={18} />
    <span class="up"><Icon name="chevron-up" size={16} /></span>
  </div>
{/if}

<div class="rec">
  {#if phase === 'preview' && captured}
    <button class="btn-x" onpointerdown={keep} onclick={cancel} aria-label={$t('msg_rec_delete')}><Icon name="trash-2" size={19} /></button>
    <div class="pill">
      <button class="play" onpointerdown={keep} onclick={toggle} aria-label={playing ? $t('msg_media_pause') : $t('msg_voice_play')}>
        <Icon name={playing ? 'pause' : 'play'} size={15} />
      </button>
      <span class="wave" aria-hidden="true">
        {#each captured.waveform as b, i (i)}<i class:on={i / captured.waveform.length < played} style="height:{Math.max(3, Math.round((b / 255) * 24))}px"></i>{/each}
      </span>
      <span class="time">{clockOf(playing ? captured.durationMs * (1 - played) : captured.durationMs)}</span>
    </div>
  {:else if locked}
    <button class="btn-x" onpointerdown={keep} onclick={cancel} aria-label={$t('msg_rec_delete')}><Icon name="trash-2" size={19} /></button>
    <span class="dot" class:on={ready}></span>
    <span class="time">{tenths(elapsed)}</span>
    <span class="level" aria-hidden="true">
      {#each bars as b, i (i)}<i style="height:{Math.max(3, Math.round(b * 24))}px"></i>{/each}
    </span>
    <button class="stop" disabled={!ready} onpointerdown={keep} onclick={stop} aria-label={$t('msg_rec_stop')}><Icon name="square" size={13} /></button>
  {:else}
    <span class="dot" class:on={ready}></span>
    <span class="time">{tenths(elapsed)}</span>
    <span class="slide" style:opacity={1 + dx / CANCEL_PX} style:transform="translateX({dx * 0.5}px)">
      <span class="nudge"><Icon name="chevron-left" size={15} /></span>{$t('msg_rec_slide_cancel')}
    </span>
  {/if}
</div>

<style>
  .rec { display: flex; align-items: center; gap: var(--sp-2); height: 40px; flex: 1; min-width: 0; padding-left: var(--sp-2); }
  .btn-x { width: 40px; height: 40px; margin-left: calc(-1 * var(--sp-2)); flex-shrink: 0; border: none; border-radius: 50%; background: none; color: var(--danger-text); display: inline-flex; align-items: center; justify-content: center; cursor: pointer; }
  .btn-x:active { background: var(--danger-bg); }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--text-3); flex-shrink: 0; }
  .dot.on { background: var(--danger); animation: blink 1.2s ease-in-out infinite; }
  @keyframes blink { 50% { opacity: 0.25; } }
  .time { font-family: var(--font-mono); font-size: var(--fs-sm); color: var(--text); font-variant-numeric: tabular-nums; flex-shrink: 0; }
  .level, .wave { flex: 1; min-width: 0; height: 28px; display: flex; align-items: center; gap: 2px; overflow: hidden; }
  .level { justify-content: flex-end; }
  .level i, .wave i { display: block; width: 3px; border-radius: 2px; background: var(--accent); flex-shrink: 0; }
  .wave i { opacity: 0.45; }
  .wave i.on { opacity: 1; }
  .slide { flex: 1; min-width: 0; display: inline-flex; align-items: center; justify-content: center; gap: 4px; padding-right: var(--sp-6); font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text-2); white-space: nowrap; }
  .nudge { display: inline-flex; animation: nudge 1.2s ease-in-out infinite; }
  @keyframes nudge { 50% { transform: translateX(-5px); } }
  .stop { width: 36px; height: 36px; flex-shrink: 0; border: none; border-radius: 50%; background: var(--danger-bg); color: var(--danger-text); display: inline-flex; align-items: center; justify-content: center; cursor: pointer; }
  .stop:disabled { opacity: 0.4; }
  .pill { flex: 1; min-width: 0; height: 40px; display: flex; align-items: center; gap: var(--sp-2); padding: 0 14px 0 4px; border-radius: 20px; background: var(--accent-tint); border: 1px solid var(--accent-tint-border); }
  .play { width: 32px; height: 32px; flex-shrink: 0; border: none; border-radius: 50%; background: var(--accent-grad); color: #fff; display: inline-flex; align-items: center; justify-content: center; cursor: pointer; }

  /* Above the composer, over the conversation. */
  .lock {
    position: absolute; right: var(--sp-3); bottom: calc(100% + 64px); z-index: 2; width: 40px; padding: 10px 0 6px;
    display: flex; flex-direction: column; align-items: center; gap: 4px;
    background: var(--surface); border: 1px solid var(--border); border-radius: 20px; box-shadow: var(--shadow); color: var(--text-2);
  }
  .up { display: inline-flex; color: var(--text-3); animation: rise 1.2s ease-in-out infinite; }
  @keyframes rise { 50% { transform: translateY(-4px); } }
  .stage {
    position: absolute; left: 0; right: 0; bottom: 100%; height: 100vh; z-index: 1; background: var(--backdrop);
    display: flex; flex-direction: column; align-items: center; justify-content: flex-end; gap: var(--sp-3); padding-bottom: var(--sp-6);
  }
  .finder { position: relative; width: min(264px, 68vw); aspect-ratio: 1; }
  .finder video { position: absolute; inset: 8px; width: calc(100% - 16px); height: calc(100% - 16px); border-radius: 50%; object-fit: cover; background: var(--surface-3); }
  .finder video.mirror { transform: scaleX(-1); }
  .ring { position: absolute; inset: 0; width: 100%; height: 100%; transform: rotate(-90deg); }
  .ring circle { fill: none; stroke-width: 4; }
  .track { stroke: color-mix(in srgb, var(--text) 18%, transparent); }
  .fill { stroke: var(--accent); stroke-linecap: round; transition: stroke-dashoffset 0.1s linear; }
  .over { position: absolute; inset: 8px; border: none; border-radius: 50%; background: none; color: #fff; display: flex; align-items: center; justify-content: center; cursor: pointer; }
  .flip { width: 44px; height: 44px; border-radius: 50%; border: 1px solid var(--border); background: var(--surface); color: var(--text); display: inline-flex; align-items: center; justify-content: center; box-shadow: var(--shadow); cursor: pointer; }
  .flip:disabled { opacity: 0.4; }
  @media (prefers-reduced-motion: reduce) { .dot.on, .nudge, .up { animation: none; } }
</style>

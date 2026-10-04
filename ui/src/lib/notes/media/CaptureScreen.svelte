<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Full-screen recorder: live preview, record/stop, then review of the take. -->

<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { CaptureError, Recorder, closeStream, levelMeter, openStream } from '$lib/notes/media/capture';
  import { fmtDuration, type MediaKey, type mediaT } from '$lib/notes/media/strings';
  import type { CameraFacing, CaptureFile, MediaKind, MediaPrefs } from '$lib/notes/media/types';

  interface Props {
    kind: MediaKind;
    prefs: MediaPrefs;
    t: ReturnType<typeof mediaT>;
    /** Render as a block inside a host container instead of a full-screen overlay. */
    embedded?: boolean;
    /** Begin recording as soon as the devices are open. */
    autostart?: boolean;
    ondone: (file: CaptureFile) => void;
    oncancel: () => void;
    onfacing: (facing: CameraFacing) => void;
  }

  let { kind, prefs, t, embedded = false, autostart = false, ondone, oncancel, onfacing }: Props = $props();

  type Phase = 'opening' | 'ready' | 'recording' | 'review' | 'error';
  let phase = $state<Phase>('opening');
  let stream = $state<MediaStream | null>(null);
  let level = $state(0);
  let elapsed = $state(0);
  let errorKey = $state<MediaKey>('err_unsupported');
  let result = $state<CaptureFile | null>(null);
  let reviewUrl = $state<string | null>(null);
  let previewEl = $state<HTMLVideoElement>();

  /** Long-press start runs once; retake and camera flip wait for the button. */
  let autostarted = false;
  let recorder: Recorder | null = null;
  let stopMeter: () => void = () => {};
  let timer: ReturnType<typeof setInterval> | undefined;

  $effect(() => {
    if (previewEl) previewEl.srcObject = stream;
  });

  async function open(facing: CameraFacing = prefs.facing) {
    release();
    phase = 'opening';
    try {
      stream = await openStream(kind, { ...prefs, facing });
      stopMeter = levelMeter(stream, (n) => (level = n));
      phase = 'ready';
      if (autostart && !autostarted) {
        autostarted = true;
        start();
      }
    } catch (e) {
      errorKey = errorKeyOf(e);
      phase = 'error';
    }
  }

  function errorKeyOf(e: unknown): MediaKey {
    if (e instanceof CaptureError) {
      if (e.code === 'denied') return 'err_denied';
      if (e.code === 'no-device') return 'err_no_device';
      if (e.code === 'busy') return 'err_busy';
    }
    return 'err_unsupported';
  }

  function start() {
    if (!stream) return;
    recorder = new Recorder(stream, kind, prefs);
    recorder.start();
    elapsed = 0;
    phase = 'recording';
    timer = setInterval(() => (elapsed = recorder?.elapsedMs() ?? 0), 250);
  }

  async function stop() {
    if (!recorder) return;
    clearInterval(timer);
    result = await recorder.stop();
    recorder = null;
    reviewUrl = URL.createObjectURL(result.blob);
    release();
    phase = 'review';
  }

  function retake() {
    dropReview();
    open();
  }

  function use() {
    const file = result;
    dropReview();
    if (file) ondone(file);
  }

  function cancel() {
    recorder?.cancel();
    recorder = null;
    dropReview();
    release();
    oncancel();
  }

  function flip() {
    const next: CameraFacing = prefs.facing === 'user' ? 'environment' : 'user';
    onfacing(next);
    open(next);
  }

  /** Stops tracks and the level meter; keeps review data. */
  function release() {
    clearInterval(timer);
    stopMeter();
    stopMeter = () => {};
    closeStream(stream);
    stream = null;
    level = 0;
  }

  function dropReview() {
    if (reviewUrl) URL.revokeObjectURL(reviewUrl);
    reviewUrl = null;
    result = null;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') cancel();
  }

  onMount(() => open());
  onDestroy(() => {
    recorder?.cancel();
    dropReview();
    release();
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="screen" class:embedded role="dialog" aria-modal={!embedded} aria-label={t(kind)}>
  <div class="top">
    <button type="button" class="ctl" onclick={cancel} aria-label={t('close')}><Icon name="x" size={22} /></button>
    <span class="timer" class:live={phase === 'recording'}>
      {#if phase === 'recording'}<span class="dot"></span>{/if}
      {fmtDuration(phase === 'review' ? (result?.durationMs ?? 0) : elapsed)}
    </span>
    {#if kind === 'video' && phase === 'ready'}
      <button type="button" class="ctl" onclick={flip} aria-label={t('switch_camera')}><Icon name="switch-camera" size={22} /></button>
    {:else}
      <span class="ctl"></span>
    {/if}
  </div>

  <div class="stage">
    {#if phase === 'error'}
      <p class="err">{t(errorKey)}</p>
    {:else if phase === 'review' && reviewUrl}
      {#if kind === 'video'}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video class="media" controls playsinline src={reviewUrl}></video>
      {:else}
        <audio class="player" controls src={reviewUrl}></audio>
      {/if}
    {:else if kind === 'video'}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video class="media" bind:this={previewEl} autoplay muted playsinline class:mirror={prefs.facing === 'user'}></video>
    {:else}
      <div class="meter" style:--lvl={level}>
        <span class="ring"></span>
        <Icon name="mic" size={40} />
      </div>
    {/if}
    {#if phase === 'opening'}<p class="hint">{t('opening')}</p>{/if}
  </div>

  <div class="bottom">
    {#if phase === 'review'}
      <button type="button" class="btn ghost" onclick={retake}><Icon name="rotate-ccw" size={18} />{t('retake')}</button>
      <button type="button" class="btn primary" onclick={use}><Icon name="check" size={18} />{t('use')}</button>
    {:else if phase === 'error'}
      <button type="button" class="btn ghost" onclick={cancel}>{t('close')}</button>
    {:else if phase === 'recording'}
      <button type="button" class="rec on" onclick={stop} aria-label={t('stop')}><span class="sq"></span></button>
    {:else}
      <button type="button" class="rec" disabled={phase !== 'ready'} onclick={start} aria-label={t('start')}><span class="circle"></span></button>
    {/if}
  </div>
</div>

<style>
  .screen {
    position: fixed;
    /* Inside the window's frame, below its title bar (core/desktop/WindowFrame.svelte). */
    inset: var(--overlay-inset, 0);
    border-radius: var(--overlay-radius, 0);
    overflow: hidden;
    z-index: var(--z-modal);
    display: flex;
    flex-direction: column;
    background: #000;
    color: #fff;
    padding: calc(var(--sp-3) + var(--sat, 0px)) var(--sp-4) calc(var(--sp-4) + var(--sab, 0px));
  }
  .screen.embedded {
    position: relative;
    inset: auto;
    z-index: auto;
    height: 420px;
    border-radius: 12px;
    padding: var(--sp-3);
  }
  .embedded .bottom { min-height: 72px; }
  .top { display: flex; align-items: center; justify-content: space-between; flex-shrink: 0; }
  .ctl {
    width: 44px;
    height: 44px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.12);
    color: #fff;
  }
  span.ctl { background: transparent; }
  .timer { display: inline-flex; align-items: center; gap: 8px; font-variant-numeric: tabular-nums; font-size: 17px; font-weight: 700; }
  .timer.live { color: #ff5252; }
  .dot { width: 10px; height: 10px; border-radius: 50%; background: #ff5252; animation: blink 1s steps(2) infinite; }
  @keyframes blink { to { opacity: 0.2; } }
  .stage { flex: 1; min-height: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--sp-3); }
  .media { width: 100%; max-height: 100%; border-radius: 16px; background: #111; object-fit: contain; }
  .media.mirror { transform: scaleX(-1); }
  .player { width: 100%; }
  .meter { position: relative; width: 160px; height: 160px; display: flex; align-items: center; justify-content: center; }
  .ring {
    position: absolute;
    inset: 0;
    border-radius: 50%;
    background: rgba(255, 82, 82, 0.35);
    transform: scale(calc(0.6 + var(--lvl, 0) * 0.6));
    transition: transform 80ms linear;
  }
  .hint, .err { margin: 0; font-size: 14px; color: rgba(255, 255, 255, 0.7); text-align: center; }
  .err { color: #ff8a80; padding: 0 var(--sp-4); }
  .bottom { display: flex; align-items: center; justify-content: center; gap: var(--sp-3); flex-shrink: 0; min-height: 88px; }
  .rec {
    width: 72px;
    height: 72px;
    padding: 0;
    border: 4px solid #fff;
    border-radius: 50%;
    background: transparent;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .rec:disabled { opacity: 0.4; }
  .circle { width: 56px; height: 56px; border-radius: 50%; background: #ff3b30; }
  .sq { width: 28px; height: 28px; border-radius: 6px; background: #ff3b30; }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-height: 44px;
    padding: 0 20px;
    border: 0;
    border-radius: 22px;
    font: inherit;
    font-size: 15px;
    font-weight: 600;
  }
  .btn.ghost { background: rgba(255, 255, 255, 0.12); color: #fff; }
  .btn.primary { background: var(--accent); color: #fff; }
</style>

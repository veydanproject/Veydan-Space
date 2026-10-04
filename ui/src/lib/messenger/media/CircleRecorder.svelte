<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Round video message: live preview, record, send or discard. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { CaptureError, LIMIT_SECS, Recorder, clockOf, closeStream, openStream, type Captured } from './capture';

  interface Props {
    oncancel: () => void;
    ondone: (c: Captured) => void;
    onerror: (code: string) => void;
  }
  let { oncancel, ondone, onerror }: Props = $props();

  let video = $state<HTMLVideoElement | null>(null);
  let stream: MediaStream | null = null;
  let recorder: Recorder | null = null;
  let facing = $state<'user' | 'environment'>('user');
  let recording = $state(false);
  let ready = $state(false);
  let elapsed = $state(0);
  let finishing = false;
  let timer: ReturnType<typeof setInterval> | null = null;

  const progress = $derived(Math.min(1, elapsed / (LIMIT_SECS.circle * 1000)));
  const R = 148;
  const C = 2 * Math.PI * R;

  function release() {
    if (timer) clearInterval(timer);
    timer = null;
    closeStream(stream);
    stream = null;
  }

  async function open() {
    ready = false;
    release();
    try {
      stream = await openStream('circle', facing);
      if (video) { video.srcObject = stream; await video.play().catch(() => {}); }
      ready = true;
    } catch (e) {
      onerror(e instanceof CaptureError ? e.code : 'unsupported');
    }
  }

  function start() {
    if (!stream || recording) return;
    recorder = new Recorder(stream, 'circle');
    recorder.start();
    recording = true;
    timer = setInterval(() => {
      elapsed = recorder?.elapsedMs() ?? 0;
      if (elapsed >= LIMIT_SECS.circle * 1000) finish();
    }, 100);
  }

  async function finish() {
    if (!recorder || finishing) return;
    finishing = true;
    try {
      const c = await recorder.stop();
      release();
      if (c.durationMs < 800 || c.blob.size === 0) oncancel();
      else ondone(c);
    } catch {
      release();
      onerror('unsupported');
    }
  }

  function cancel() {
    recorder?.cancel();
    release();
    oncancel();
  }

  async function flip() {
    if (recording) return;
    facing = facing === 'user' ? 'environment' : 'user';
    await open();
  }

  onMount(() => {
    open();
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') cancel(); };
    window.addEventListener('keydown', key);
    return () => { window.removeEventListener('keydown', key); if (!finishing) recorder?.cancel(); release(); };
  });
</script>

<div class="overlay" role="dialog" aria-modal="true" aria-label={$t('msg_rec_circle')}>
  <div class="stage">
    <svg class="ring" viewBox="0 0 304 304" aria-hidden="true">
      <circle cx="152" cy="152" r={R} class="track" />
      <circle cx="152" cy="152" r={R} class="fill" stroke-dasharray={C} stroke-dashoffset={C * (1 - progress)} />
    </svg>
    <!-- svelte-ignore a11y_media_has_caption -->
    <video bind:this={video} class:mirror={facing === 'user'} muted playsinline></video>
  </div>
  <div class="time" class:on={recording}>{clockOf(elapsed)} / {clockOf(LIMIT_SECS.circle * 1000)}</div>
  <div class="controls">
    <button class="side" onclick={cancel} aria-label={$t('msg_media_cancel')}><Icon name="x" size={22} /></button>
    {#if recording}
      <button class="main stop" onclick={finish} aria-label={$t('msg_composer_send')}><Icon name="send" size={24} /></button>
    {:else}
      <button class="main" disabled={!ready} onclick={start} aria-label={$t('msg_rec_start')}><span class="rec-dot"></span></button>
    {/if}
    <button class="side" disabled={recording} onclick={flip} aria-label={$t('msg_rec_flip')}><Icon name="switch-camera" size={22} /></button>
  </div>
</div>

<style>
  .overlay {
    position: fixed; inset: var(--overlay-inset, 0); border-radius: var(--overlay-radius, 0); overflow: hidden; z-index: var(--z-modal, 1000); background: rgba(8, 8, 14, 0.94);
    display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--sp-4);
    padding: var(--sp-4) var(--sp-4) calc(var(--sp-5) + var(--sab, 0px));
  }
  .stage { position: relative; width: min(304px, 80vw); aspect-ratio: 1; }
  video { position: absolute; inset: 8px; width: calc(100% - 16px); height: calc(100% - 16px); border-radius: 50%; object-fit: cover; background: #111; }
  video.mirror { transform: scaleX(-1); }
  .ring { position: absolute; inset: 0; width: 100%; height: 100%; transform: rotate(-90deg); }
  .ring circle { fill: none; stroke-width: 4; }
  .track { stroke: rgba(255, 255, 255, 0.14); }
  .fill { stroke: var(--accent); stroke-linecap: round; transition: stroke-dashoffset 0.1s linear; }
  .time { font-family: var(--font-mono); font-size: var(--fs-sm); color: rgba(255, 255, 255, 0.6); }
  .time.on { color: #fff; }
  .controls { display: flex; align-items: center; gap: var(--sp-6); }
  .side { width: 52px; height: 52px; border: none; border-radius: 50%; background: rgba(255, 255, 255, 0.1); color: #fff; display: inline-flex; align-items: center; justify-content: center; cursor: pointer; }
  .side:disabled { opacity: 0.3; }
  .main { width: 76px; height: 76px; border: 4px solid #fff; border-radius: 50%; background: none; display: inline-flex; align-items: center; justify-content: center; cursor: pointer; color: #fff; }
  .main:disabled { opacity: 0.4; }
  .main.stop { background: var(--accent-grad); border-color: transparent; }
  .rec-dot { width: 52px; height: 52px; border-radius: 50%; background: var(--danger); }
</style>

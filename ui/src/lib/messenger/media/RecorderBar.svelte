<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Voice recording in place of the composer row: timer, live level,
  cancel and send. Recording starts as soon as the bar is shown. It takes
  the field's pill (48 high, as wide) with the level right after the time,
  and its send is the main button's: 48, in its place (ui-unify 02).
-->
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

  let stream: MediaStream | null = null;
  let recorder: Recorder | null = null;
  let elapsed = $state(0);
  let bars = $state<number[]>([]);
  let ready = $state(false);
  let finishing = false;
  let timer: ReturnType<typeof setInterval> | null = null;

  function cleanup() {
    if (timer) clearInterval(timer);
    timer = null;
    closeStream(stream);
    stream = null;
  }

  async function finish() {
    if (!recorder || finishing) return;
    finishing = true;
    try {
      const c = await recorder.stop();
      cleanup();
      // Shorter than a breath is a mis-tap, not a message.
      if (c.durationMs < 600 || c.blob.size === 0) oncancel();
      else ondone(c);
    } catch {
      cleanup();
      onerror('unsupported');
    }
  }

  function cancel() {
    recorder?.cancel();
    cleanup();
    oncancel();
  }

  onMount(() => {
    let gone = false;
    openStream('voice')
      .then((s) => {
        if (gone) { closeStream(s); return; }
        stream = s;
        recorder = new Recorder(s, 'voice', (l) => { bars = [...bars.slice(-39), l]; });
        recorder.start();
        ready = true;
        timer = setInterval(() => {
          elapsed = recorder?.elapsedMs() ?? 0;
          if (elapsed >= LIMIT_SECS.voice * 1000) finish();
        }, 200);
      })
      .catch((e) => onerror(e instanceof CaptureError ? e.code : 'unsupported'));
    return () => { gone = true; if (!finishing) recorder?.cancel(); cleanup(); };
  });

  const keep = (e: Event) => e.preventDefault();
</script>

<div class="rec">
  <button class="btn-x" onpointerdown={keep} onclick={cancel} title={$t('msg_media_cancel')} aria-label={$t('msg_media_cancel')}>
    <Icon name="trash-2" size={17} />
  </button>
  <span class="dot" class:on={ready}></span>
  <span class="time">{clockOf(elapsed)}</span>
  <span class="level" aria-hidden="true">
    {#each bars as b, i (i)}<i style="height:{Math.max(3, Math.round(b * 26))}px"></i>{/each}
  </span>
  <button class="send" disabled={!ready} onpointerdown={keep} onclick={finish} title={$t('msg_composer_send')} aria-label={$t('msg_composer_send')}>
    <Icon name="send" size={18} />
  </button>
</div>

<style>
  .rec { display: flex; align-items: center; gap: var(--sp-2); min-height: 40px; flex: 1; min-width: 0; }
  .btn-x { width: 40px; height: 40px; flex-shrink: 0; border: none; border-radius: 50%; background: none; color: var(--danger-text); display: inline-flex; align-items: center; justify-content: center; cursor: pointer; }
  .btn-x:hover { background: var(--danger-bg); }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--text-3); flex-shrink: 0; }
  .dot.on { background: var(--danger); animation: blink 1.2s ease-in-out infinite; }
  @keyframes blink { 50% { opacity: 0.25; } }
  .time { font-family: var(--font-mono); font-size: var(--fs-sm); color: var(--text); min-width: 42px; }
  .level { flex: 1; min-width: 0; height: 28px; display: flex; align-items: center; justify-content: flex-start; gap: 2px; overflow: hidden; }
  .level i { display: block; width: 3px; border-radius: 2px; background: var(--accent); flex-shrink: 0; }
  /* The row's padding is 4 above and below: the 48 reaches over it, as the main button does. */
  .send { width: 48px; height: 48px; margin: calc(-1 * var(--sp-1)) 0; flex-shrink: 0; border: none; border-radius: 50%; cursor: pointer; background: var(--accent-grad); color: #fff; display: inline-flex; align-items: center; justify-content: center; }
  .send:hover:not(:disabled) { filter: brightness(1.1); }
  .send:disabled { opacity: 0.45; }
  .btn-x:focus-visible, .send:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  @media (pointer: coarse) { .btn-x { width: 44px; height: 44px; } }
</style>

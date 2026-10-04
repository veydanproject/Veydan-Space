<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Voice message: play button, outline that fills while playing, time. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { clockOf } from './capture';
  import { playback } from './playback.svelte';

  interface Props {
    id: string;
    src: string | null;
    durationMs: number;
    waveform: number[];
    out: boolean;
    /** Called when the file is not on the device yet. */
    onneed: () => void;
    busy: boolean;
  }
  let { id, src, durationMs, waveform, out, onneed, busy }: Props = $props();

  let audio = $state<HTMLAudioElement | null>(null);
  let playing = $state(false);
  let position = $state(0);
  let measured = $state(0);
  let wantPlay = false;

  // Recorded webm often reports no duration; trust the message, then the element.
  const total = $derived(durationMs > 0 ? durationMs : measured);
  const progress = $derived(total > 0 ? Math.min(1, position / total) : 0);
  const bars = $derived(waveform.length ? waveform : Array.from({ length: 32 }, (_, i) => 60 + ((i * 53) % 120)));

  // Only one voice message sounds at a time.
  $effect(() => { if (playback.current !== id && playing) audio?.pause(); });

  // The file arrived after the user pressed play.
  $effect(() => { if (src && wantPlay && audio) { wantPlay = false; queueMicrotask(toggle); } });

  function toggle() {
    if (!src) { wantPlay = true; onneed(); return; }
    if (!audio) return;
    if (playing) { audio.pause(); return; }
    playback.current = id;
    audio.play().catch(() => {});
  }

  function seek(e: MouseEvent) {
    if (!audio || !src || !total) return;
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const ratio = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width));
    audio.currentTime = (ratio * total) / 1000;
    position = ratio * total;
  }
</script>

<div class="voice" class:out>
  <button class="play" onclick={toggle} disabled={busy && !src} aria-label={playing ? $t('msg_media_pause') : $t('msg_voice_play')}>
    {#if busy && !src}<span class="spin"><Icon name="loader" size={18} /></span>
    {:else}<Icon name={playing ? 'square' : 'play'} size={playing ? 14 : 17} />{/if}
  </button>
  <div class="body">
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="wave" role="slider" tabindex="-1" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(progress * 100)} onclick={seek}>
      {#each bars as b, i (i)}
        <i class:done={i / bars.length < progress} style="height:{Math.max(3, Math.round((b / 255) * 24))}px"></i>
      {/each}
    </div>
    <span class="time">{playing || position > 0 ? clockOf(position) : clockOf(total)}</span>
  </div>
  {#if src}
    <audio bind:this={audio} {src} preload="metadata"
      onplay={() => (playing = true)}
      onpause={() => (playing = false)}
      onended={() => { playing = false; position = 0; }}
      ontimeupdate={() => (position = (audio?.currentTime ?? 0) * 1000)}
      onloadedmetadata={() => { const d = audio?.duration ?? 0; if (Number.isFinite(d)) measured = d * 1000; }}
    ></audio>
  {/if}
</div>

<style>
  .voice { display: flex; align-items: center; gap: var(--sp-2); min-width: 220px; max-width: 320px; }
  .play {
    width: 40px; height: 40px; flex-shrink: 0; border: none; border-radius: 50%; cursor: pointer;
    background: var(--accent); color: #fff; display: inline-flex; align-items: center; justify-content: center;
  }
  .play:disabled { opacity: 0.6; }
  .spin :global(svg) { animation: spin 1.1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .body { display: flex; flex-direction: column; gap: 3px; min-width: 0; flex: 1; }
  .wave { display: flex; align-items: center; gap: 2px; height: 26px; cursor: pointer; overflow: hidden; }
  .wave i { display: block; flex: 1 1 0; min-width: 2px; max-width: 4px; border-radius: 2px; background: var(--text-3); opacity: 0.55; }
  .wave i.done { background: var(--accent); opacity: 1; }
  .time { font-size: var(--fs-2xs); color: var(--text-3); font-family: var(--font-mono); }
  @media (pointer: coarse) { .play { width: 44px; height: 44px; } }
</style>

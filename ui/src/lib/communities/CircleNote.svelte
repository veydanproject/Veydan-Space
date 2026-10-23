<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A video circle of a post or a comment, as a chat shows one (MediaBubble):
  round, its first frame once it is here, a tap plays it with its sound and
  stops it, one sound at a time (the messenger's `playback`). Before it is
  here: a dark round with the play button and its length; a tap fetches it
  (`onneed`) and plays it once it comes.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { NO_POSTER, clockOf, playback, showFirstFrame } from '$lib/messenger/seams/shared';

  interface Props {
    /** The message: whose sound plays now. */
    id: string;
    /** Where it plays from; `null` while it is not here. */
    src: string | null;
    durationMs: number;
    /** A small frame the message carries, shown until it is here. */
    thumb?: string | null;
    busy?: boolean;
    /** It is not here: fetch it. */
    onneed: () => void;
  }
  let { id, src, durationMs, thumb = null, busy = false, onneed }: Props = $props();

  let video = $state<HTMLVideoElement | null>(null);
  let playing = $state(false);
  let position = $state(0);
  /** The user asked for the sound; the silent first frame is not that. */
  let wanted = false;
  /** Pressed before it was here: it plays once it comes. */
  let waiting = $state(false);

  // Another sound started: this one stops.
  $effect(() => { if (playback.current !== id && playing) video?.pause(); });

  function play() {
    if (!video) return;
    wanted = true;
    video.muted = false;
    playback.current = id;
    if (video.ended) video.currentTime = 0;
    video.play().catch(() => {});
  }

  function toggle() {
    if (!src) { waiting = true; onneed(); return; }
    if (playing) { wanted = false; video?.pause(); return; }
    play();
  }

  function loaded() {
    if (!video) return;
    if (waiting) { waiting = false; play(); return; }
    showFirstFrame(video, () => wanted);
  }

  const left = $derived(playing || position > 0 ? Math.max(0, durationMs - position) : durationMs);
</script>

<button class="circle" onclick={toggle} aria-label={playing ? $t('msg_media_pause') : $t('msg_voice_play')}>
  {#if src}
    <!-- svelte-ignore a11y_media_has_caption -->
    <video bind:this={video} {src} poster={NO_POSTER} playsinline preload="auto" onloadeddata={loaded}
      ontimeupdate={() => (position = (video?.currentTime ?? 0) * 1000)}
      onplay={() => (playing = !video?.muted)} onpause={() => (playing = false)}
      onended={() => { playing = false; wanted = false; position = 0; }}></video>
  {:else if thumb}
    <img src={thumb} alt="" />
  {/if}
  {#if !playing}
    <span class="play">{#if busy && !src}<span class="spin"><Icon name="loader" size={22} /></span>{:else}<Icon name="play" size={26} />{/if}</span>
  {/if}
  {#if durationMs > 0}<span class="len">{clockOf(left)}</span>{/if}
</button>

<style>
  .circle {
    position: relative; display: block; width: 220px; height: 220px; max-width: 100%; padding: 0; border: none; border-radius: 50%;
    overflow: hidden; cursor: pointer; flex-shrink: 0;
    background: radial-gradient(circle at 35% 30%, color-mix(in srgb, var(--accent) 45%, #111), #0b0b0f 75%);
  }
  .circle video, .circle img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .play { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; color: #fff; background: rgba(0, 0, 0, 0.28); }
  .spin { display: inline-flex; }
  .spin :global(svg) { animation: spin 1.1s linear infinite; }
  .len {
    position: absolute; left: 50%; bottom: 12px; transform: translateX(-50%); padding: 1px 8px; border-radius: var(--radius-pill);
    background: rgba(0, 0, 0, 0.5); color: #fff; font-family: var(--font-mono); font-size: var(--fs-2xs); line-height: 18px;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>

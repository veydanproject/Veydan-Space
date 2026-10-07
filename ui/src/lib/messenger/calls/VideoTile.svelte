<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One video of a call on a canvas: the frames of `track` come from the
  runtime on a channel for as long as the tile is there (the subscription is
  taken back when it goes, and made anew for a new call), and are drawn as
  they come (calls/video.ts). The canvas holds the picture upright and, for
  my own camera, mirrored; it fills the tile cropped or whole (`fit`).

  `live`: a frame came within the last moment. A video turned off stops
  its frames, so the screen around shows a placeholder instead of the last
  picture frozen.
-->
<script lang="ts" module>
  export interface TileInfo {
    width: number;
    height: number;
    fps: number;
  }
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import { messengerApi, type VideoTrack } from '../api';
  import { FpsMeter, FrameAcks, FrameRenderer, fitFor, frameSeq, parseFrame, shownSize, type I420Frame } from './video';

  interface Props {
    track: VideoTrack;
    /** The call the frames belong to: another call, another subscription. */
    callId: string;
    /** Mirrored left to right: my own face, as in a mirror. */
    mirror?: boolean;
    /** `auto`: cropped to fill when the picture stands as the tile does, whole otherwise. */
    fit?: 'cover' | 'contain' | 'auto';
    live?: boolean;
    /** The size the picture shows at and the frames a second, for the screen to tell. */
    info?: TileInfo | null;
  }
  let { track, callId, mirror = false, fit = 'auto', live = $bindable(false), info = $bindable(null) }: Props = $props();

  /** No frame for this long: the video is taken as stopped. */
  const STALE_MS = 1200;

  let canvas = $state<HTMLCanvasElement | null>(null);
  let boxW = $state(0);
  let boxH = $state(0);
  const objectFit = $derived(fit === 'auto' ? fitFor(info, { width: boxW, height: boxH }) : fit);
  /**
   * WebGL came on this tile's canvas but cannot draw: the canvas is made
   * anew (one that held WebGL gives no 2D context) and drawn on in 2D.
   */
  let noGl = $state(false);

  $effect(() => {
    const el = canvas;
    const id = callId;
    const which = track;
    // A canvas the key took away (its context let go) is not drawn on: the
    // new one comes next.
    if (!el || !el.isConnected || !id) return;
    // A new canvas comes with `noGl` (the key below): it is read untracked.
    const renderer = new FrameRenderer(el, { webgl: !untrack(() => noGl) });
    if (renderer.broken && !untrack(() => noGl)) {
      renderer.destroy();
      noGl = true;
      return;
    }
    const meter = new FpsMeter();
    let lastAt = 0;
    let gone = false;
    let sub: number | null = null;
    // The runtime sends the next frame once this one is acknowledged: every
    // message but the last, drawn or not (calls/video.ts `FrameAcks`).
    const acks = new FrameAcks((n, seq) => messengerApi.calls.videoAck(n, seq));

    const onframe = (data: ArrayBuffer) => {
      if (gone) return;
      const frame = parseFrame(data);
      if (frame === 'end') {
        lastAt = 0;
        live = false;
        return;
      }
      const seq = frameSeq(data);
      try {
        if (frame) show(frame);
      } finally {
        if (seq != null) acks.took(seq);
      }
    };

    const show = (frame: I420Frame) => {
      if (!renderer.draw(frame, mirror)) {
        // Nothing shown (the context is lost): the frame does not count, so
        // the tile goes not live and the screen shows its placeholder. A
        // context that came back unable to draw needs a new canvas.
        if (renderer.broken && !noGl) noGl = true;
        return;
      }
      const now = performance.now();
      meter.frame(now);
      lastAt = now;
      if (!live) live = true;
      const shown = shownSize(frame);
      if (!info || info.width !== shown.width || info.height !== shown.height) info = { ...shown, fps: meter.fps(now) };
    };

    // The counts of the screen change twice a second, not with every frame.
    const timer = setInterval(() => {
      const now = performance.now();
      const fresh = lastAt > 0 && now - lastAt < STALE_MS;
      if (live !== fresh) live = fresh;
      if (info) {
        const fps = meter.fps(now);
        if (fps !== info.fps) info = { ...info, fps };
      }
    }, 500);

    messengerApi.calls.videoSubscribe(which, onframe).then(
      (n) => {
        if (gone) messengerApi.calls.videoUnsubscribe(n).catch(() => {});
        else {
          sub = n;
          acks.ready(n);
        }
      },
      () => { /* no engine, or the call is over: the tile stays empty */ },
    );

    return () => {
      gone = true;
      acks.close();
      clearInterval(timer);
      if (sub != null) messengerApi.calls.videoUnsubscribe(sub).catch(() => {});
      renderer.destroy();
      live = false;
      info = null;
    };
  });
</script>

<!-- A canvas for each call and kind of drawing: the renderer lets its WebGL
     context go when it is done, and a canvas gives its context once. -->
<div class="tile" bind:clientWidth={boxW} bind:clientHeight={boxH}>
  {#key `${track}:${callId}:${noGl}`}
    <canvas bind:this={canvas} width="2" height="2" style:object-fit={objectFit} class:shown={live}></canvas>
  {/key}
</div>

<style>
  .tile { position: absolute; inset: 0; overflow: hidden; background: #000; }
  canvas { display: block; width: 100%; height: 100%; opacity: 0; transition: opacity 160ms ease-out; }
  canvas.shown { opacity: 1; }
</style>

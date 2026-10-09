<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One video of a call on a canvas: the frames of `track` come from the
  runtime on a channel for as long as the tile is there (the subscription is
  taken back when it goes, and made anew for a new call), and are drawn as
  they come (calls/video.ts). The canvas holds the picture upright and, for
  my own camera, mirrored; it fills the tile cropped or whole (`fit`).

  `live`: there is a picture to show: a frame came (since the video went
  on) and its stream did not end. A pause of the frames is ordinary (the
  node changes the layer of a simulcast at a key frame, the way's bandwidth
  floats): the last picture stays on the canvas through it and `live`
  stays, so nothing around it moves. `lost`: no frame for
  `VIDEO_LOST_MS` (or the stream ended): the picture is dimmed, and the
  screen around says so where the call says the video is on. The clock of
  `lost` is the tile's and starts again with a new tile; a seat of a group
  call, whose camera the room does not tell of, is judged where its clock
  outlives its tiles (groupCallStore `showing`), from what the tile tells
  (`onpicture`, `onended`, by the m-line of its subscription). `on`: the
  video is on as the call says (my camera, the peer's in a call of two);
  while it is off nothing is held, so a picture shows again only with a
  frame that came after it went on.
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
  import { FpsMeter, FrameAcks, FrameRenderer, PictureWatch, fitFor, frameSeq, parseFrame, shownSize, type I420Frame } from './video';

  interface Props {
    track: VideoTrack;
    /** The call the frames belong to: another call, another subscription. */
    callId: string;
    /**
     * A seat of a group call: the m-line of its video
     * (`GroupParticipant.video_mid`); `track` is not read then.
     */
    mid?: string | null;
    /** Mirrored left to right: my own face, as in a mirror. */
    mirror?: boolean;
    /** `auto`: cropped to fill when the picture stands as the tile does, whole otherwise. */
    fit?: 'cover' | 'contain' | 'auto';
    /** The video is on as the call says; without a word of the call (a seat of a group call), on. */
    on?: boolean;
    /** A picture is there to show: the last one stays through a pause of the frames. */
    live?: boolean;
    /** No frame for `VIDEO_LOST_MS`, or the stream ended. */
    lost?: boolean;
    /** The size the picture shows at and the frames a second, for the screen to tell. */
    info?: TileInfo | null;
    /**
     * A seat of a group call: a picture of the video with the m-line `mid`
     * was drawn. The m-line is the one this tile's subscription has, not
     * the one of a seat the tile was given since.
     */
    onpicture?: (mid: string) => void;
    /** A seat of a group call: the stream of the video with the m-line `mid` ended. */
    onended?: (mid: string) => void;
  }
  let {
    track, callId, mid = null, mirror = false, fit = 'auto', on = true,
    live = $bindable(false), lost = $bindable(false), info = $bindable(null), onpicture, onended,
  }: Props = $props();

  /** Whether there is a picture and whether its signal is lost (calls/video.ts `PictureWatch`). */
  const watch = new PictureWatch(performance.now());
  /** What the watch says, to the screen around; written only when it changes. */
  const sync = () => {
    if (live !== watch.live) live = watch.live;
    if (lost !== watch.lost) lost = watch.lost;
  };

  // The video went off: nothing is held, its last picture is let go. On
  // again: the frames are waited for anew, the old ones do not count.
  $effect(() => {
    watch.turn(on, performance.now());
    untrack(sync);
  });

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
    const seat = mid;
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
    watch.restart(performance.now());
    untrack(sync);
    let gone = false;
    let sub: number | null = null;
    // The runtime sends the next frame once this one is acknowledged: every
    // message but the last, drawn or not (calls/video.ts `FrameAcks`).
    const acks = new FrameAcks((n, seq) => messengerApi.calls.videoAck(n, seq));

    const onframe = (data: ArrayBuffer) => {
      if (gone) return;
      const frame = parseFrame(data);
      if (frame === 'end') {
        watch.end();
        sync();
        if (seat) onended?.(seat);
        return;
      }
      const seq = frameSeq(data);
      try {
        // A video the call says is off draws nothing: its picture would
        // stay behind for when it goes on again.
        if (frame && watch.wanted) show(frame);
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
      watch.frame(now);
      sync();
      if (seat) onpicture?.(seat);
      const shown = shownSize(frame);
      if (!info || info.width !== shown.width || info.height !== shown.height) info = { ...shown, fps: meter.fps(now) };
    };

    // The counts of the screen change twice a second, not with every frame.
    const timer = setInterval(() => {
      const now = performance.now();
      watch.tick(now);
      sync();
      if (info) {
        const fps = meter.fps(now);
        if (fps !== info.fps) info = { ...info, fps };
      }
    }, 500);

    (seat ? messengerApi.groupCalls.videoSubscribe(seat, onframe) : messengerApi.calls.videoSubscribe(which, onframe)).then(
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
      watch.restart(performance.now());
      sync();
      info = null;
    };
  });
</script>

<!-- A canvas for each call and kind of drawing: the renderer lets its WebGL
     context go when it is done, and a canvas gives its context once. -->
<div class="tile" bind:clientWidth={boxW} bind:clientHeight={boxH}>
  {#key `${mid ?? track}:${callId}:${noGl}`}
    <canvas bind:this={canvas} width="2" height="2" style:object-fit={objectFit} class:shown={live} class:dim={live && lost}></canvas>
  {/key}
</div>

<style>
  .tile { position: absolute; inset: 0; overflow: hidden; background: #000; }
  canvas { display: block; width: 100%; height: 100%; opacity: 0; transition: opacity 160ms ease-out; }
  canvas.shown { opacity: 1; }
  /* The signal lost: the last picture stays, dimmed, under the words of the screen around. */
  canvas.dim { filter: brightness(0.45) saturate(0.6); transition: opacity 160ms ease-out, filter 400ms ease-out; }
</style>

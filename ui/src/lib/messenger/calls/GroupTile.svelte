<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One seat of a group call: its video while its camera is on, its face
  with a halo that follows its voice otherwise. My own camera is on as my
  call says; another's as its own word of state says (calls/group.ts
  `seatVideoWord`): while it is on, a pause of its frames keeps the last
  picture, the tile and the screen as they are; missing for
  `VIDEO_LOST_MS`, the last picture stays dimmed under "No signal" (its
  face says it before a first picture). A seat that says nothing of its
  camera (an older client) is taken as on with its first picture and as
  off once its pictures have been missing that long (groupCallStore
  `showing`). Over it: its name, "speaking", a crossed microphone when
  its microphone is off (mine; another's by its word: calls/group.ts
  `seatMuted`). A seat whose word of identity was not
  checked yet has no name and is not heard (messenger-wire §10): it shows
  as such. A tap picks the seat for the large place (and again lets go).
  My own seat shows what I send (the frames of my camera or screen, by
  the m-line of "me": calls/group.ts `MY_VIDEO_MID`), mirrored as a face
  in a mirror is.

  The tile asks the node for the layer of the seat's video its size needs
  (calls/group.ts `layerFor`), once the size settles.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import type { GroupParticipant } from '../api';
  import { nameStore } from '../groups/names.svelte';
  import CallFace from './CallFace.svelte';
  import CallIcon from './CallIcon.svelte';
  import { MY_VIDEO_MID, layerFor, seatMuted, seatSendsVideo, seatVideoWord, tileWait } from './group';
  import { groupCallStore } from './groupCallStore.svelte';
  import VideoTile, { type TileInfo } from './VideoTile.svelte';

  interface Props {
    p: GroupParticipant;
    callId: string;
    /** How loud the seat is, 0..1. */
    level?: number;
    /** The large place: a larger face, the size of the picture said. */
    big?: boolean;
    /** Picked by a tap for the large place. */
    pinned?: boolean;
    /** My own seat: my microphone is off. */
    muted?: boolean;
    /** My own seat: my camera (or screen) goes. */
    sending?: boolean;
    /** My own seat: shown mirrored (a face, not a screen or the world of a back camera). */
    mirror?: boolean;
    /** A tap: pick the seat, or let it go. */
    onpick?: () => void;
  }
  let { p, callId, level = 0, big = false, pinned = false, muted = false, sending = false, mirror = false, onpick }: Props = $props();

  const name = $derived(p.me ? $t('msg_call_video_you') : p.verified && p.npub ? nameStore.label(p.npub) : $t('msg_gcall_unverified'));
  /** My own face is my picture and my initials, not those of "You". */
  const face = $derived({ name: p.me && p.npub ? nameStore.label(p.npub) : name, picture: p.npub ? nameStore.picture(p.npub) : null, seed: p.npub ?? `seat:${p.id}` });
  /** The m-line of the pictures to show: another's video, or mine while it goes. */
  const video = $derived(p.me ? (sending ? MY_VIDEO_MID : null) : p.verified && p.video_mid ? p.video_mid : null);
  const silent = $derived(seatMuted(p, { muted }));

  /** A picture to show (the last one stays through a pause), and no frame for long. */
  let live = $state(false);
  let lost = $state(false);
  let info = $state<TileInfo | null>(null);
  let w = $state(0);
  let h = $state(0);
  /** A small tile (a row under the large one, a crowded grid): a smaller face, its notes as icons. */
  const compact = $derived(h > 0 && h < 130);

  // Another's pictures (calls/group.ts `seatSendsVideo` for a seat that
  // says nothing of its camera): every picture drawn and the end of the stream go to
  // the store by the m-line of the subscription that brought them, and the
  // store keeps the seat's clock (groupCallStore `showing`). Not this tile's
  // `live` and `lost`: they start anew with every tile (one that moved
  // between the large place, the row and the grid), and the large tile,
  // given another seat, holds those of the last one for a moment.
  const picture = (mid: string) => groupCallStore.seatPicture(mid);
  const ended = (mid: string) => groupCallStore.seatEnded(mid);
  /** Another's own word of its video: on, off, or `null` (not known: judged by its pictures). */
  const word = $derived(p.me ? null : seatVideoWord(p));
  /** The seat's video is on for this tile: mine as my call says, another's by its word or, without one, as the store takes it. */
  const on = $derived(p.me ? sending : seatSendsVideo(p, { video_local: false }, (seat) => !!groupCallStore.showing[seat]));
  /** The video is on as the seat itself says (mine: my call): its tile is held through any pause, and says when the pause is long. */
  const told = $derived(p.me ? sending : word === true);
  /**
   * No picture here (the tile made anew, the camera on again): dark while
   * the seat's pictures come this moment, its face otherwise, saying it
   * waits for one or, by the seat's clock (groupCallStore `stale`), that
   * none came for long (calls/group.ts `tileWait`).
   */
  const wait = $derived(
    tileWait({ me: p.me, told, live, lost, flowing: !!groupCallStore.flowing[p.id], stale: !!groupCallStore.stale[p.id] }),
  );
  const waiting = $derived(wait === 'waiting' || wait === 'lost');
  /** The video is on and its pictures have been missing for long: the last one stays, dimmed, and says so. */
  const noSignal = $derived(told && !!video && live && lost);

  // The layer for the size of the tile, once it stood still for a moment:
  // a window being dragged larger asks once, not at every pixel. My own
  // pictures come from my camera, not the node: no layer to ask for.
  $effect(() => {
    const mid = video;
    const width = w;
    const height = h;
    if (!mid || p.me || width <= 0 || height <= 0) return;
    const seat = p.id;
    const timer = setTimeout(() => void groupCallStore.setLayer(seat, layerFor(width, height, window.devicePixelRatio || 1)), 400);
    return () => clearTimeout(timer);
  });
</script>

<div class="tile" class:big class:compact class:speaking={p.speaking && p.verified} class:pinned class:unverified={!p.verified} class:pickable={!!onpick}
  bind:clientWidth={w} bind:clientHeight={h} role="button" tabindex={onpick ? 0 : -1} aria-label={name} aria-pressed={pinned}
  onclick={onpick} onkeydown={(e) => { if (onpick && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); onpick(); } }}>
  {#if video}
    <!-- Off by the seat's word: nothing is held, a picture comes back only with a frame after it went on. -->
    <VideoTile track={p.me ? 'local' : 'remote'} mid={video} {callId} mirror={p.me && mirror} fit="cover" on={word !== false} bind:live bind:lost bind:info
      onpicture={p.me ? undefined : picture} onended={p.me ? undefined : ended} />
  {/if}
  <!-- A camera on whose tile has no picture yet while its pictures come this
       moment (the tile is only made anew): dark for that moment, not its
       face, or the tile would blink. Its face until a first picture
       otherwise, saying it waits for one, or that none came for long. -->
  {#if !video || !on || waiting}
    <span class="face">
      <CallFace peer={face} size={big && !compact ? 96 : compact ? 34 : 56} level={p.verified ? level : 0} />
      {#if !p.verified}<span class="checking" title={$t('msg_gcall_checking')}><CallIcon name="shield-question" size={13} />{#if !compact}{$t('msg_gcall_checking')}{/if}</span>{/if}
      {#if !p.me && waiting && video}<span class="checking mine" role="status" title={$t(wait === 'lost' ? 'msg_call_video_lost' : 'msg_call_video_waiting')}><CallIcon name={wait === 'lost' ? 'video-off' : p.screen ? 'screen-share' : 'video'} size={13} />{#if !compact}{$t(wait === 'lost' ? 'msg_call_video_lost' : 'msg_call_video_waiting')}{/if}</span>{/if}
      {#if p.me && sending}<span class="checking mine" title={$t(groupCallStore.screen ? 'msg_call_sharing' : 'msg_gcall_camera_on')}><CallIcon name={groupCallStore.screen ? 'screen-share' : 'video'} size={13} />{#if !compact}{$t(groupCallStore.screen ? 'msg_call_sharing' : 'msg_gcall_camera_on')}{/if}</span>{/if}
    </span>
  {/if}
  <span class="label">
    {#if p.verified && !p.me}<span class="ok" title={$t('msg_gcall_verified_hint')}><CallIcon name="shield-check" size={12} /></span>{/if}
    <span class="name">{name}</span>
    {#if silent}<span class="mark" title={$t('msg_call_muted')}><CallIcon name="mic-off" size={12} /></span>{/if}
    {#if p.speaking && p.verified}<span class="talk" title={$t('msg_gcall_speaking')}><CallIcon name="speaking" size={12} />{#if !compact}{$t('msg_gcall_speaking')}{/if}</span>{/if}
  </span>
  {#if noSignal}<span class="lost" role="status"><CallIcon name="video-off" size={13} />{#if !compact}{$t('msg_call_video_lost')}{/if}</span>{/if}
  {#if big && live && !lost && info}<span class="info">{$t('msg_call_video_info', { size: `${info.width}×${info.height}`, fps: String(info.fps) })}</span>{/if}
  {#if pinned}<span class="pin" title={$t('msg_gcall_pinned')}><CallIcon name="pip" size={12} /></span>{/if}
</div>

<style>
  .tile {
    position: relative; width: 100%; height: 100%; min-width: 0; min-height: 0; overflow: hidden; padding: 0; margin: 0;
    border-radius: 12px; border: 2px solid transparent; background: #1a1e24; color: #fff; font: inherit; cursor: default;
    display: block; text-align: left; box-sizing: border-box; transition: border-color 160ms ease-out;
  }
  .tile.pickable { cursor: pointer; }
  .tile.speaking { border-color: var(--success); }
  .tile.pinned { border-color: rgba(255, 255, 255, 0.7); }
  .tile.speaking.pinned { border-color: var(--success); }
  .tile:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .face {
    position: absolute; inset: 0; background: #1a1e24; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px;
    z-index: 1; padding-bottom: 18px;
  }
  .unverified .face { opacity: 0.75; }
  .checking {
    display: inline-flex; align-items: center; gap: 4px; padding: 2px 8px; border-radius: var(--radius-pill);
    background: rgba(255, 255, 255, 0.1); color: rgba(255, 255, 255, 0.8); font-size: var(--fs-2xs); white-space: nowrap;
  }
  .checking.mine { background: color-mix(in srgb, var(--accent) 45%, rgba(0, 0, 0, 0.4)); color: #fff; }
  .label {
    position: absolute; z-index: 2; left: 6px; bottom: 6px; right: 6px; display: flex; align-items: center; gap: 5px; min-width: 0;
    pointer-events: none;
  }
  .label > * { flex-shrink: 0; }
  .name {
    flex-shrink: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    padding: 2px 8px; border-radius: var(--radius-pill); background: rgba(0, 0, 0, 0.55); font-size: var(--fs-xs); font-weight: var(--fw-semibold);
  }
  .ok, .mark, .pin {
    display: inline-flex; align-items: center; justify-content: center; width: 20px; height: 20px; border-radius: 50%;
    background: rgba(0, 0, 0, 0.55); color: #fff;
  }
  .ok { color: #7ee2a8; pointer-events: auto; cursor: help; }
  .mark { color: #ffb4a8; }
  .talk {
    display: inline-flex; align-items: center; gap: 3px; padding: 2px 7px; border-radius: var(--radius-pill);
    background: var(--success); color: #fff; font-size: var(--fs-2xs); font-weight: var(--fw-bold); white-space: nowrap;
  }
  .pin { position: absolute; z-index: 2; top: 6px; right: 6px; }
  .lost {
    position: absolute; z-index: 2; top: 50%; left: 50%; transform: translate(-50%, -50%); display: inline-flex; align-items: center; gap: 5px;
    padding: 3px 10px; border-radius: var(--radius-pill); background: rgba(0, 0, 0, 0.55); color: rgba(255, 255, 255, 0.9);
    font-size: var(--fs-2xs); white-space: nowrap; pointer-events: none;
  }
  .info {
    position: absolute; z-index: 2; top: 6px; left: 6px; padding: 2px 8px; border-radius: var(--radius-pill);
    background: rgba(0, 0, 0, 0.5); color: rgba(255, 255, 255, 0.85); font-size: var(--fs-2xs); font-variant-numeric: tabular-nums;
    pointer-events: none;
  }
  .big .name { font-size: var(--fs-sm); }
  .compact .face { flex-direction: row; gap: 6px; padding-bottom: 22px; }
  .compact .checking { padding: 3px; }
  .compact .talk { padding: 2px 4px; }
  .compact .ok { display: none; }
</style>

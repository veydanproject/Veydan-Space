<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The videos of a call of two: the peer's large, mine small in a corner; a
  tap on the small one swaps them. Until the call talks my own camera is
  the large one (the peer's has nothing to show yet). While a video is off (or its frames have
  not come yet) its place shows the face and why: a frozen last picture
  would look like a live one. My camera is mirrored, as people expect to see
  themselves; a phone's back camera (it shows the world) and my screen are
  not.

  It fills the box it is put in: the desk's video window, the phone's call
  screen. Both subscriptions live as long as the stage, whatever is on.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import type { CallView } from '../api';
  import CallFace from './CallFace.svelte';
  import CallIcon from './CallIcon.svelte';
  import type { CallPeer } from './peer';
  import VideoTile, { type TileInfo } from './VideoTile.svelte';
  import { mirrorsLocal } from './video';

  interface Props {
    call: CallView;
    peer: CallPeer;
    /** The phone's screen: the small video under the top bar, larger faces. */
    phone?: boolean;
    /** How loud the peer is, for the halo of the face while their video is off. */
    level?: number;
    /** What the call is doing, said in the peer's place until it talks. */
    status?: string;
  }
  let { call, peer, phone = false, level = 0, status = '' }: Props = $props();

  let swapped = $state(false);
  let remoteLive = $state(false);
  let localLive = $state(false);
  let remoteInfo = $state<TileInfo | null>(null);
  let localInfo = $state<TileInfo | null>(null);

  const localOn = $derived(call.video_local);
  const remoteOn = $derived(call.video_remote);
  // My video gone: the peer's goes back to the large place.
  $effect(() => { if (!localOn && swapped) swapped = false; });

  /** Ringing or connecting with my camera on: my own picture, large. */
  const preview = $derived(call.phase !== 'active' && localOn && !remoteOn);
  const main = $derived(swapped || preview ? 'local' : 'remote');
  const mainLive = $derived(main === 'remote' ? remoteOn && remoteLive : localOn && localLive);
  const mainInfo = $derived(main === 'remote' ? remoteInfo : localInfo);
  /** The small place: mine while my video goes; the peer's (or their face) when swapped. */
  const small = $derived(!preview && (swapped || localOn));
  const smallLive = $derived(main === 'remote' ? localOn && localLive : remoteOn && remoteLive);

  const mainNote = $derived(
    main === 'remote'
      ? (call.phase !== 'active' && status ? status : remoteOn ? $t('msg_call_video_waiting') : $t('msg_call_video_peer_off'))
      : (localOn ? $t('msg_call_video_waiting') : $t('msg_call_video_my_off')),
  );
  const info = $derived(mainLive && mainInfo ? $t('msg_call_video_info', { size: `${mainInfo.width}×${mainInfo.height}`, fps: String(mainInfo.fps) }) : '');
</script>

<div class="stage" class:phone>
  <!-- The large place. -->
  <div class="slot main" class:swapped>
    {#if !mainLive}
      <div class="placeholder">
        <CallFace {peer} size={phone ? 112 : 72} level={main === 'remote' ? level : 0} />
        <span class="note">{main === 'local' ? $t('msg_call_video_you') + ' · ' : ''}{mainNote}</span>
      </div>
    {/if}
  </div>

  <!-- The two tiles move between the places; their canvases stay where they are drawn. -->
  <div class="tile-place" class:big={main === 'remote'} class:small={main !== 'remote'} class:hidden={main !== 'remote' && !small}>
    <VideoTile track="remote" callId={call.call_id} fit={main === 'remote' ? 'auto' : 'cover'} bind:live={remoteLive} bind:info={remoteInfo} />
  </div>
  <div class="tile-place" class:big={main === 'local'} class:small={main !== 'local'} class:hidden={main !== 'local' && !small}>
    <VideoTile track="local" callId={call.call_id} mirror={mirrorsLocal(call)} fit={main === 'local' ? 'auto' : 'cover'} bind:live={localLive} bind:info={localInfo} />
  </div>

  {#if small}
    <button class="pip" class:dark={!smallLive} onclick={() => (swapped = !swapped)} title={$t('msg_call_video_swap')} aria-label={$t('msg_call_video_swap')}>
      {#if !smallLive}
        {#if main === 'remote'}
          <span class="pip-note"><CallIcon name="video-off" size={16} /></span>
        {:else}
          <CallFace {peer} size={phone ? 44 : 36} {level} />
        {/if}
      {/if}
      {#if call.video_screen && main === 'remote'}<span class="pip-tag"><CallIcon name="screen-share" size={11} /><span class="pip-tag-text">{$t('msg_call_sharing')}</span></span>{/if}
    </button>
  {/if}

  {#if info}<span class="info">{info}</span>{/if}
</div>

<style>
  .stage { position: absolute; inset: 0; overflow: hidden; background: #0b0d10; color: #fff; }
  .slot.main { position: absolute; inset: 0; z-index: 1; display: flex; align-items: center; justify-content: center; }
  .placeholder { display: flex; flex-direction: column; align-items: center; gap: var(--sp-3); padding: var(--sp-4); text-align: center; }
  .note { font-size: var(--fs-sm); color: rgba(255, 255, 255, 0.72); }

  .tile-place { position: absolute; z-index: 0; transition: inset 220ms cubic-bezier(0.2, 0.8, 0.3, 1), border-radius 220ms; overflow: hidden; }
  .tile-place.big { inset: 0; }
  /* The small place: top right, clear of the buttons at the bottom; on a
     phone under its top bar. */
  .tile-place.small {
    z-index: 3; pointer-events: none; border-radius: 10px;
    inset: var(--pip-at); width: var(--pip-w); aspect-ratio: var(--pip-ratio);
  }
  .tile-place.hidden { opacity: 0; }
  .stage { --pip-w: clamp(96px, 26%, 220px); --pip-ratio: 16 / 10; --pip-at: 10px 10px auto auto; }
  .stage.phone { --pip-w: 104px; --pip-ratio: 3 / 4; --pip-at: calc(var(--sat, 0px) + 64px) 14px auto auto; }

  .pip {
    position: absolute; z-index: 4; inset: var(--pip-at); width: var(--pip-w); aspect-ratio: var(--pip-ratio);
    border-radius: 10px; border: 1.5px solid rgba(255, 255, 255, 0.55); padding: 0; margin: 0; cursor: pointer;
    background: transparent; box-shadow: 0 6px 22px rgba(0, 0, 0, 0.45);
    display: flex; align-items: center; justify-content: center;
    transition: border-color var(--dur-fast) var(--ease);
  }
  .pip.dark { background: #1c2026; }
  @media (hover: hover) { .pip:hover { border-color: #fff; } }
  .pip:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .pip-note { color: rgba(255, 255, 255, 0.7); display: inline-flex; }
  .pip-tag {
    position: absolute; left: 4px; right: 4px; bottom: 4px; display: inline-flex; align-items: center; justify-content: center; gap: 4px;
    padding: 2px 6px; border-radius: var(--radius-pill); background: rgba(0, 0, 0, 0.6); color: #fff;
    font-size: var(--fs-2xs); white-space: nowrap;
  }
  .pip-tag-text { min-width: 0; overflow: hidden; text-overflow: ellipsis; }

  .info {
    position: absolute; z-index: 2; left: 10px; top: 10px; padding: 2px 8px; border-radius: var(--radius-pill);
    background: rgba(0, 0, 0, 0.45); color: rgba(255, 255, 255, 0.85); font-size: var(--fs-2xs); font-variant-numeric: tabular-nums;
    pointer-events: none;
  }
  .phone .info { top: auto; bottom: calc(var(--sab, 0px) + 150px); left: 50%; transform: translateX(-50%); }
  @media (prefers-reduced-motion: reduce) { .tile-place { transition: none; } }
</style>

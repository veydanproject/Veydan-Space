<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The video of a call on a computer: a window over the page in the bottom
  right corner, or (enlarged) over the whole page.
  It is there while either side's video goes. Its bar: the microphone, my
  camera, the next camera when there is more than one, my screen (with a
  choice of screens and windows when there are several), the size, the end.

  The small window can be dragged where it covers nothing; the place and
  the size are kept in this browser only.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { ScreenInfo } from '../api';
  import CallIcon from './CallIcon.svelte';
  import { callStore } from './callStore.svelte';
  import { callPeer } from './peer';
  import { statusText } from './status';
  import VideoStage from './VideoStage.svelte';
  import { clampShift } from './video';

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  const call = $derived(callStore.video ? callStore.call : null);
  const status = $derived(call ? statusText(call, null, callStore.now, tr) : '');
  const peer = $derived(call ? callPeer(call) : null);
  const cameraOn = $derived(!!call?.video_local && !call.video_screen);
  const sharing = $derived(!!call?.video_screen);

  const KEY = 'messenger.call.video';
  let dx = $state(0);
  let dy = $state(0);
  let big = $state(false);
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? 'null') as { dx?: number; dy?: number; big?: boolean } | null;
    if (saved && Number.isFinite(saved.dx) && Number.isFinite(saved.dy)) { dx = saved.dx!; dy = saved.dy!; }
    if (saved?.big) big = true;
  } catch { /* no storage: the window starts in its corner */ }

  function keep() {
    try { localStorage.setItem(KEY, JSON.stringify({ dx, dy, big })); } catch { /* for this window only */ }
  }

  // The cameras are asked once per call: the switch is there only with two or more.
  let askedCameras = '';
  $effect(() => {
    if (!call || askedCameras === call.call_id) return;
    askedCameras = call.call_id;
    void callStore.loadCameras();
  });

  let win = $state<HTMLElement | null>(null);
  let drag: { x: number; y: number; dx: number; dy: number } | null = null;

  /** The shift held on the page as it is now (the window's size follows the page's). */
  function held(x: number, y: number) {
    // offsetWidth/Height: the layout size, not the size the opening animation scales.
    return clampShift({ dx: x, dy: y }, { width: win!.offsetWidth, height: win!.offsetHeight }, { width: innerWidth, height: innerHeight });
  }

  /**
   * The kept place is brought onto the page when the window shows, when it
   * comes back from the whole page and when the page changes size; the kept
   * value itself changes only with a drag, so a larger page gets it back.
   */
  function fit() {
    if (!win || big) return;
    const s = held(dx, dy);
    if (s.dx !== dx) dx = s.dx;
    if (s.dy !== dy) dy = s.dy;
  }
  $effect(() => {
    if (win && !big) untrack(fit);
  });

  function down(e: PointerEvent) {
    if (big || e.button !== 0 || (e.target as HTMLElement).closest('button, .menu')) return;
    drag = { x: e.clientX, y: e.clientY, dx, dy };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function move(e: PointerEvent) {
    if (!drag || !win) return;
    ({ dx, dy } = held(drag.dx + e.clientX - drag.x, drag.dy + e.clientY - drag.y));
  }

  function up() {
    if (!drag) return;
    drag = null;
    keep();
  }

  function resize() {
    big = !big;
    keep();
  }

  // The choice of what to show, when there is more than one thing.
  let picking = $state<ScreenInfo[] | null>(null);
  let none = $state(false);

  async function screen() {
    if (sharing) { picking = null; await callStore.stopScreen(); return; }
    if (picking) { picking = null; return; }
    none = false;
    const list = await callStore.loadScreens();
    if (!list) return;
    if (list.length > 1) picking = list;
    else if (list.length === 1) await callStore.shareScreen(list[0].id);
    else none = true;
  }

  async function pick(s: ScreenInfo) {
    picking = null;
    await callStore.shareScreen(s.id);
  }

  $effect(() => { if (!call) { picking = null; none = false; } });
</script>

<svelte:window onkeydown={(e) => { if (e.key === 'Escape' && picking) picking = null; }} onresize={fit} />

{#if call && peer}
  <div
    class="win" class:big bind:this={win} style:translate={big ? null : `${dx}px ${dy}px`}
    role="group" aria-label={peer.name}
    onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up}
  >
    <VideoStage {call} {peer} {status} level={callStore.level} route rtt={callStore.stats?.rtt_ms ?? null} />

    <div class="bar">
      <button class="ctl" class:off={call.muted} disabled={callStore.busy || call.phase === 'outgoing'} onclick={() => callStore.toggleMute()}
        title={$t(call.muted ? 'msg_call_unmute' : 'msg_call_mute')} aria-label={$t(call.muted ? 'msg_call_unmute' : 'msg_call_mute')} aria-pressed={call.muted}>
        <CallIcon name={call.muted ? 'mic-off' : 'mic'} size={17} />
      </button>
      <button class="ctl" class:off={!cameraOn} disabled={callStore.busy} onclick={() => callStore.toggleCamera()}
        title={$t(cameraOn ? 'msg_call_camera_off' : 'msg_call_camera_on')} aria-label={$t(cameraOn ? 'msg_call_camera_off' : 'msg_call_camera_on')} aria-pressed={cameraOn}>
        <CallIcon name={cameraOn ? 'video' : 'video-off'} size={17} />
      </button>
      {#if cameraOn && callStore.cameras.length > 1}
        <button class="ctl" disabled={callStore.busy} onclick={() => callStore.switchCamera()} title={$t('msg_call_camera_switch')} aria-label={$t('msg_call_camera_switch')}>
          <Icon name="switch-camera" size={17} />
        </button>
      {/if}
      <span class="menu-wrap">
        <button class="ctl" class:active={sharing} disabled={callStore.busy} onclick={screen}
          title={$t(sharing ? 'msg_call_screen_stop' : 'msg_call_screen_share')} aria-label={$t(sharing ? 'msg_call_screen_stop' : 'msg_call_screen_share')}
          aria-pressed={sharing} aria-haspopup="menu" aria-expanded={!!picking}>
          <CallIcon name={sharing ? 'screen-share-off' : 'screen-share'} size={17} />
        </button>
        {#if picking}
          <div class="menu" role="menu" aria-label={$t('msg_call_screen_pick')}>
            <div class="menu-title">{$t('msg_call_screen_pick')}</div>
            {#each picking as s (s.id)}
              <button class="item" role="menuitem" onclick={() => pick(s)}>
                <Icon name={s.window ? 'browser' : 'monitor'} size={14} />
                <span class="item-title">{s.title}</span>
                <span class="item-kind">{$t(s.window ? 'msg_call_screen_kind_window' : 'msg_call_screen_kind_screen')}</span>
              </button>
            {/each}
          </div>
        {:else if none}
          <div class="menu note" role="status">{$t('msg_call_screen_none')}</div>
        {/if}
      </span>
      <button class="ctl" onclick={resize} title={$t(big ? 'msg_call_video_shrink' : 'msg_call_video_expand')} aria-label={$t(big ? 'msg_call_video_shrink' : 'msg_call_video_expand')}>
        <Icon name={big ? 'minimize-2' : 'maximize-2'} size={16} />
      </button>
      <button class="ctl end" disabled={callStore.ending} onclick={() => callStore.hangUp()} title={$t('msg_call_end')} aria-label={$t('msg_call_end')}>
        <CallIcon name="hangup" size={18} />
      </button>
    </div>
  </div>
{/if}

<style>
  .win {
    position: fixed; z-index: 70; right: 16px; bottom: 16px;
    width: clamp(300px, 34vw, 460px); aspect-ratio: 16 / 10;
    border-radius: var(--radius-lg); overflow: hidden; background: #0b0d10;
    border: 1px solid var(--border-strong); box-shadow: var(--shadow-lg);
    cursor: grab; touch-action: none; user-select: none;
    animation: rise 240ms cubic-bezier(0.2, 0.9, 0.3, 1.1);
  }
  .win:active { cursor: grabbing; }
  /* Enlarged: the whole page, where full-window overlays go (inside the
     window's frame, under its title bar, which holds the capsule of the
     call). `--overlay-inset` is a list of lengths: it is taken whole. */
  .win.big {
    inset: var(--overlay-inset, 0); width: auto; aspect-ratio: auto; cursor: default;
    border-radius: var(--overlay-radius, 0); border: none;
  }
  .bar {
    position: absolute; z-index: 5; left: 50%; bottom: 10px; transform: translateX(-50%);
    display: flex; align-items: center; gap: 6px; padding: 5px; border-radius: var(--radius-pill);
    background: rgba(12, 14, 18, 0.62); backdrop-filter: blur(10px); -webkit-backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.12);
    opacity: 0; transition: opacity var(--dur-fast) var(--ease);
  }
  .win:hover .bar, .win:focus-within .bar, .win.big .bar { opacity: 1; }
  @media (hover: none) { .bar { opacity: 1; } }
  .ctl {
    width: 36px; height: 36px; border-radius: 50%; border: none; cursor: pointer;
    display: inline-flex; align-items: center; justify-content: center;
    background: rgba(255, 255, 255, 0.14); color: #fff;
    transition: background var(--dur-fast) var(--ease);
  }
  .ctl:hover:not(:disabled) { background: rgba(255, 255, 255, 0.26); }
  .ctl.off { background: #fff; color: #111; }
  .ctl.off:hover:not(:disabled) { background: #e8e8e8; }
  .ctl.active { background: var(--accent); }
  .ctl.end { width: 46px; border-radius: var(--radius-pill); background: var(--danger); }
  .ctl.end:hover:not(:disabled) { background: var(--danger); filter: brightness(1.1); }
  .ctl:disabled { opacity: 0.5; cursor: default; }
  .ctl:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .menu-wrap { position: relative; display: inline-flex; }
  .menu {
    position: absolute; bottom: calc(100% + 10px); left: 50%; transform: translateX(-50%);
    width: max-content; min-width: 220px; max-width: min(360px, 80vw); max-height: 260px; overflow-y: auto;
    padding: 6px; border-radius: var(--radius); background: var(--surface); color: var(--text);
    border: 1px solid var(--border-strong); box-shadow: var(--shadow-lg); cursor: default;
  }
  .menu.note { padding: 8px 12px; font-size: var(--fs-xs); color: var(--text-2); min-width: 0; }
  .menu-title { padding: 4px 8px 6px; font-size: var(--fs-2xs); color: var(--text-3); text-transform: uppercase; letter-spacing: 0.4px; }
  .item {
    width: 100%; display: flex; align-items: center; gap: 8px; padding: 7px 8px; border: none; border-radius: var(--radius-sm);
    background: none; color: inherit; font: inherit; font-size: var(--fs-sm); cursor: pointer; text-align: left;
  }
  .item:hover, .item:focus-visible { background: var(--surface-2); outline: none; }
  .item-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .item-kind { font-size: var(--fs-2xs); color: var(--text-3); }
  @keyframes rise { from { opacity: 0; transform: translateY(14px) scale(0.97); } }
  @media (prefers-reduced-motion: reduce) { .win { animation: none; } }
</style>

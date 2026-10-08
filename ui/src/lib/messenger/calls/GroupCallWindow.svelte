<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The room of a group call on a computer: a window over the page in the
  bottom right corner, or (enlarged) over the whole page. Its top: the
  group, the clock, how many are in, the node the call goes through; the
  seats in the middle (GroupStage), who is in beside them (GroupRoster);
  its bar: the microphone, my camera, my screen, the list, the size, Leave.
  Folded, it gives way to the capsule at the top of the window
  (GroupCallPanel). After the room is over it says how for a moment.

  The small window can be dragged by its top; the place and the size are
  kept in this browser only.
-->
<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { t, locale, countKey } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { ScreenInfo } from '../api';
  import { chatStore } from '../chats/chatStore.svelte';
  import { groupStore } from '../groups/groupStore.svelte';
  import CallFace from './CallFace.svelte';
  import CallIcon from './CallIcon.svelte';
  import { callStore } from './callStore.svelte';
  import { groupStatusText, nodeHost, peopleIn } from './group';
  import { groupCallStore } from './groupCallStore.svelte';
  import GroupRoster from './GroupRoster.svelte';
  import GroupStage from './GroupStage.svelte';
  import { clampShift } from './video';
  import { callErrorText } from './words';

  interface Props {
    /** Shows the chat of the group (the messenger page with it open). */
    onchat: (chatId: string) => void;
  }
  let { onchat }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  const call = $derived(groupCallStore.call);
  const over = $derived(call ? null : groupCallStore.over);
  const shown = $derived(call ?? over?.call ?? null);
  const open = $derived(!!shown && (groupCallStore.shown || !call));
  const group = $derived(shown ? groupStore.groups[shown.group_id] ?? null : null);
  const name = $derived(group?.name || (shown ? chatStore.chats.find((c) => c.id === shown.chat_id)?.title : '') || '');
  const face = $derived({ name, picture: group?.picture || null, seed: shown?.group_id ?? '' });
  const status = $derived(groupStatusText(call, over?.how ?? null, groupCallStore.now, tr));
  const people = $derived(call ? peopleIn(call) : 0);
  const count = $derived(people ? $t(countKey('msg_gcall_people', people, $locale), { n: String(people) }) : '');
  const cameraOn = $derived(!!call?.video_local && !groupCallStore.screen);
  const sharing = $derived(!!call?.video_local && groupCallStore.screen);
  const error = $derived(groupCallStore.error && call ? callErrorText(groupCallStore.error, tr) : '');
  const node = $derived(call?.node ? nodeHost(call.node) : '');

  const KEY = 'messenger.gcall.window';
  let dx = $state(0);
  let dy = $state(0);
  let big = $state(false);
  let roster = $state(false);
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? 'null') as { dx?: number; dy?: number; big?: boolean; roster?: boolean } | null;
    if (saved && Number.isFinite(saved.dx) && Number.isFinite(saved.dy)) { dx = saved.dx!; dy = saved.dy!; }
    if (saved?.big) big = true;
    if (saved?.roster) roster = true;
  } catch { /* no storage: the window starts in its corner */ }

  function keep() {
    try { localStorage.setItem(KEY, JSON.stringify({ dx, dy, big, roster })); } catch { /* for this window only */ }
  }

  let win = $state<HTMLElement | null>(null);
  let drag: { x: number; y: number; dx: number; dy: number } | null = null;

  function held(x: number, y: number) {
    return clampShift({ dx: x, dy: y }, { width: win!.offsetWidth, height: win!.offsetHeight }, { width: innerWidth, height: innerHeight });
  }

  function fit() {
    if (!win || big) return;
    const s = held(dx, dy);
    if (s.dx !== dx) dx = s.dx;
    if (s.dy !== dy) dy = s.dy;
  }
  // Back inside the page whenever the window takes another size: it
  // opens, the list of who is in widens it (to the left, from its right
  // edge), the list goes with the room.
  const wide = $derived(roster && !!call);
  $effect(() => {
    void wide;
    if (win && !big) untrack(fit);
  });

  function down(e: PointerEvent) {
    // Held by its top only: the seats below take taps of their own.
    const at = e.target as HTMLElement;
    if (big || e.button !== 0 || !at.closest('.top') || at.closest('button')) return;
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

  function resize() { big = !big; keep(); }
  /** The list on or off; the window, wider or narrower, is kept inside the page and where it ends up. */
  function toggleRoster() {
    roster = !roster;
    void tick().then(() => { fit(); keep(); });
  }

  // The choice of what to show, when there is more than one thing.
  let picking = $state<ScreenInfo[] | null>(null);
  let none = $state(false);

  async function screen() {
    if (sharing) { picking = null; await groupCallStore.stopScreen(); return; }
    if (picking) { picking = null; return; }
    none = false;
    const list = await callStore.loadScreens();
    if (!list) return;
    if (list.length > 1) picking = list;
    else if (list.length === 1) await groupCallStore.shareScreen(list[0].id);
    else none = true;
  }

  async function pick(s: ScreenInfo) {
    picking = null;
    await groupCallStore.shareScreen(s.id);
  }

  $effect(() => { if (!call) { picking = null; none = false; } });
</script>

<svelte:window onkeydown={(e) => { if (e.key === 'Escape' && picking) picking = null; }} onresize={fit} />

{#if open && shown}
  <div class="win" class:big class:with-roster={roster && !!call} bind:this={win} style:translate={big ? null : `${dx}px ${dy}px`}
    role="group" aria-label={name} onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up}>
    <span class="sr" aria-live="polite">{call ? '' : status}</span>
    <header class="top">
      <CallFace peer={face} size={28} />
      <span class="title">
        <span class="name">{name}</span>
        <span class="sub">
          <span class="phase" class:clock={call?.phase === 'in_room'}>{status}</span>
          {#if count}<span class="dotsep">·</span><span>{count}</span>{/if}
        </span>
      </span>
      {#if node}
        <span class="chip" title={$t('msg_gcall_via_node_hint')}><CallIcon name="server" size={11} />{$t('msg_gcall_via_node', { node })}</span>
      {/if}
      <span class="chip quiet" title={$t('msg_gcall_e2e_hint')}><Icon name="lock" size={11} />{$t('msg_call_e2e')}</span>
      {#if call}
        <button class="hbtn" onclick={() => onchat(call.chat_id)} title={$t('msg_call_open_chat')} aria-label={$t('msg_call_open_chat')}><Icon name="message-circle" size={15} /></button>
        <button class="hbtn" onclick={() => (groupCallStore.shown = false)} title={$t('msg_gcall_fold')} aria-label={$t('msg_gcall_fold')}><Icon name="minus" size={15} /></button>
        <button class="hbtn" onclick={resize} title={$t(big ? 'msg_call_video_shrink' : 'msg_call_video_expand')} aria-label={$t(big ? 'msg_call_video_shrink' : 'msg_call_video_expand')}>
          <Icon name={big ? 'minimize-2' : 'maximize-2'} size={15} />
        </button>
      {/if}
    </header>

    <div class="body">
      <div class="stage">
        {#if call}
          <GroupStage {call} />
        {:else if over}
          <div class="over"><CallFace peer={face} size={72} /><span>{status}</span></div>
        {/if}
      </div>
      {#if roster && call}
        <aside class="side"><GroupRoster {call} dark /></aside>
      {/if}
    </div>

    {#if error}<div class="error" role="alert">{error}</div>{/if}

    {#if call}
      <div class="bar">
        <button class="ctl" class:off={call.muted} disabled={groupCallStore.busy} onclick={() => groupCallStore.toggleMute()}
          title={$t(call.muted ? 'msg_call_unmute' : 'msg_call_mute')} aria-label={$t(call.muted ? 'msg_call_unmute' : 'msg_call_mute')} aria-pressed={call.muted}>
          <CallIcon name={call.muted ? 'mic-off' : 'mic'} size={17} />
        </button>
        <button class="ctl" class:off={!cameraOn} disabled={groupCallStore.busy} onclick={() => groupCallStore.toggleCamera()}
          title={$t(cameraOn ? 'msg_call_camera_off' : 'msg_call_camera_on')} aria-label={$t(cameraOn ? 'msg_call_camera_off' : 'msg_call_camera_on')} aria-pressed={cameraOn}>
          <CallIcon name={cameraOn ? 'video' : 'video-off'} size={17} />
        </button>
        <span class="menu-wrap">
          <button class="ctl" class:active={sharing} disabled={groupCallStore.busy} onclick={screen}
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
        <button class="ctl" class:active={roster} onclick={toggleRoster} title={$t('msg_gcall_roster')} aria-label={$t('msg_gcall_roster')} aria-pressed={roster}>
          <Icon name="users" size={17} />
        </button>
        <button class="ctl end" disabled={groupCallStore.ending} onclick={() => groupCallStore.leave()} title={$t('msg_gcall_leave')} aria-label={$t('msg_gcall_leave')}>
          <CallIcon name="hangup" size={18} />
        </button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .win {
    position: fixed; z-index: 70; right: 16px; bottom: 16px;
    width: clamp(420px, 44vw, 680px); height: clamp(300px, 34vw, 480px);
    display: flex; flex-direction: column;
    border-radius: var(--radius-lg); overflow: hidden; background: #0b0d10; color: #fff;
    border: 1px solid var(--border-strong); box-shadow: var(--shadow-lg);
    animation: rise 240ms cubic-bezier(0.2, 0.9, 0.3, 1.1);
  }
  .win.with-roster:not(.big) { width: clamp(560px, 58vw, 860px); }
  .win.big { inset: var(--overlay-inset, 0); width: auto; height: auto; border-radius: var(--overlay-radius, 0); border: none; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  .top {
    display: flex; align-items: center; gap: 8px; padding: 8px 8px 6px 10px; flex-shrink: 0;
    cursor: grab; touch-action: none; user-select: none;
  }
  .big .top { cursor: default; padding: 10px 12px; }
  .top:active { cursor: grabbing; }
  .big .top:active { cursor: default; }
  .title { display: flex; flex-direction: column; min-width: 0; flex: 1; line-height: 1.25; }
  .name { font-weight: var(--fw-bold); font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { display: flex; gap: 5px; font-size: var(--fs-2xs); color: rgba(255, 255, 255, 0.7); white-space: nowrap; overflow: hidden; }
  .phase.clock { font-variant-numeric: tabular-nums; color: #7ee2a8; font-weight: var(--fw-semibold); }
  .dotsep { opacity: 0.6; }
  .chip {
    display: inline-flex; align-items: center; gap: 4px; padding: 2px 8px; border-radius: var(--radius-pill); flex-shrink: 0;
    background: rgba(255, 255, 255, 0.1); color: rgba(255, 255, 255, 0.85); font-size: var(--fs-2xs); white-space: nowrap; cursor: help;
  }
  .chip.quiet { color: rgba(255, 255, 255, 0.65); }
  .win:not(.big) .chip.quiet { display: none; }
  .hbtn {
    width: 28px; height: 28px; border-radius: 50%; border: none; cursor: pointer; flex-shrink: 0;
    display: inline-flex; align-items: center; justify-content: center; background: rgba(255, 255, 255, 0.1); color: #fff;
  }
  .hbtn:hover { background: rgba(255, 255, 255, 0.22); }
  .hbtn:focus-visible, .ctl:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .body { flex: 1; min-height: 0; display: flex; }
  .stage { position: relative; flex: 1; min-width: 0; }
  .side { width: 260px; flex-shrink: 0; border-left: 1px solid rgba(255, 255, 255, 0.1); display: flex; flex-direction: column; min-height: 0; background: #12161b; }
  .win:not(.big) .side { width: 220px; }
  .over { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; font-size: var(--fs-sm); color: rgba(255, 255, 255, 0.85); }
  .error {
    position: absolute; z-index: 6; left: 50%; bottom: 66px; transform: translateX(-50%); width: max-content; max-width: 80%;
    padding: 6px 10px; border-radius: var(--radius-sm); font-size: var(--fs-xs); text-align: center;
    background: var(--danger-bg); border: 1px solid var(--danger-border); color: var(--danger-text); box-shadow: var(--shadow);
  }
  .bar { display: flex; align-items: center; justify-content: center; gap: 8px; padding: 8px; flex-shrink: 0; }
  .ctl {
    width: 38px; height: 38px; border-radius: 50%; border: none; cursor: pointer;
    display: inline-flex; align-items: center; justify-content: center;
    background: rgba(255, 255, 255, 0.14); color: #fff; transition: background var(--dur-fast) var(--ease);
  }
  .ctl:hover:not(:disabled) { background: rgba(255, 255, 255, 0.26); }
  .ctl.off { background: #fff; color: #111; }
  .ctl.off:hover:not(:disabled) { background: #e8e8e8; }
  .ctl.active { background: var(--accent); }
  .ctl.end { width: 52px; border-radius: var(--radius-pill); background: var(--danger); }
  .ctl.end:hover:not(:disabled) { background: var(--danger); filter: brightness(1.1); }
  .ctl:disabled { opacity: 0.5; cursor: default; }
  .menu-wrap { position: relative; display: inline-flex; }
  .menu {
    position: absolute; bottom: calc(100% + 10px); left: 50%; transform: translateX(-50%); z-index: 8;
    width: max-content; min-width: 220px; max-width: min(360px, 80vw); max-height: 260px; overflow-y: auto;
    padding: 6px; border-radius: var(--radius); background: var(--surface); color: var(--text);
    border: 1px solid var(--border-strong); box-shadow: var(--shadow-lg);
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

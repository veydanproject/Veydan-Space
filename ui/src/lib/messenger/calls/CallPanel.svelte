<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The call under way on a computer: a capsule in the middle of the
  window's top bar, where it covers no chat, whatever page is open. The peer, the phase or the clock, the way
  the sound goes and its round trip; the microphone, my camera (the video
  itself is VideoWindow's), the chat, the end. After the end it says how
  the call ended for a moment, then goes.

  It can be dragged aside when it covers something; the place is kept in
  this browser only.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import CallFace from './CallFace.svelte';
  import CallIcon from './CallIcon.svelte';
  import { callStore } from './callStore.svelte';
  import { clampPanelShift } from './panel';
  import { callPeer } from './peer';
  import { liveText, statusText } from './status';
  import { callErrorText } from './words';

  interface Props {
    /** Shows the chat of the call (the messenger page with it open). */
    onchat: (chatId: string) => void;
  }
  let { onchat }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  const call = $derived(callStore.call);
  /** What is on the card: the call (a ringing one has a card of its own), or the one that just ended. */
  const shown = $derived(call ? (call.phase === 'incoming' ? null : call) : (callStore.ended?.call ?? null));
  const peer = $derived(shown ? callPeer(shown) : null);
  const status = $derived(statusText(call, callStore.ended, callStore.now, tr));
  /** What a screen reader is told: the phase as it changes, never the clock (it would be read out every second). */
  const live = $derived(liveText(call, callStore.ended, tr));
  const talking = $derived(call?.phase === 'active');
  const cameraOn = $derived(!!call?.video_local && !call.video_screen);
  const rtt = $derived(talking ? callStore.stats?.rtt_ms : undefined);
  const error = $derived(callStore.error && call ? callErrorText(callStore.error, tr) : '');

  const KEY = 'messenger.call.panel';
  let dx = $state(0);
  let dy = $state(0);
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? 'null') as { dx?: number; dy?: number } | null;
    if (saved && Number.isFinite(saved.dx) && Number.isFinite(saved.dy)) { dx = saved.dx!; dy = saved.dy!; }
  } catch { /* no storage: the card starts in its place */ }

  let card = $state<HTMLElement | null>(null);
  let drag: { x: number; y: number; dx: number; dy: number } | null = null;

  function down(e: PointerEvent) {
    if (e.button !== 0 || (e.target as HTMLElement).closest('button')) return;
    drag = { x: e.clientX, y: e.clientY, dx, dy };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  /** The shift held in the window as it is now. offset*: the layout box, without the opening animation's transform. */
  function held(x: number, y: number) {
    return clampPanelShift({ dx: x, dy: y }, { width: card!.offsetWidth, height: card!.offsetHeight, top: card!.offsetTop }, { width: innerWidth, height: innerHeight });
  }

  /**
   * The kept place is brought into the window when the capsule shows, when
   * its width changes (the phase, the chips) and when the window changes
   * size; the place kept in storage changes only with a drag, so the next
   * call in a larger window gets it back.
   */
  function fit() {
    if (!card || drag) return;
    const s = held(dx, dy);
    if (s.dx !== dx) dx = s.dx;
    if (s.dy !== dy) dy = s.dy;
  }
  $effect(() => {
    const el = card;
    if (!el) return;
    untrack(fit);
    const seen = new ResizeObserver(() => fit());
    seen.observe(el);
    return () => seen.disconnect();
  });

  function move(e: PointerEvent) {
    if (!drag || !card) return;
    ({ dx, dy } = held(drag.dx + e.clientX - drag.x, drag.dy + e.clientY - drag.y));
  }

  function up() {
    if (!drag) return;
    drag = null;
    try { localStorage.setItem(KEY, JSON.stringify({ dx, dy })); } catch { /* kept for this window only */ }
  }
</script>

<svelte:window onresize={fit} />

{#if shown && peer}
  <div
    class="card" class:ended={!call} bind:this={card} style:translate="{dx}px {dy}px"
    role="group" aria-label={peer.name} onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up}
  >
    <span class="sr" aria-live="polite">{live}</span>
    <CallFace {peer} size={30} ringing={call?.phase === 'outgoing'} level={talking ? callStore.level : 0} />
    <div class="who">
      <span class="name">{peer.name}</span>
      <span class="status">
        <span class="phase" class:clock={talking}>{status}</span>
        {#if call?.via}
          <span class="via" class:relay={call.via === 'relay'} title={$t(call.via === 'relay' ? 'msg_call_via_relay_hint' : 'msg_call_via_direct_hint')}>
            <CallIcon name={call.via} size={11} />{$t(call.via === 'relay' ? 'msg_call_via_relay' : 'msg_call_via_direct')}
          </span>
        {/if}
        {#if rtt != null}<span class="rtt">{$t('msg_call_rtt', { ms: String(rtt) })}</span>{/if}
        {#if call?.muted}<span class="muted-mark" title={$t('msg_call_muted')}><CallIcon name="mic-off" size={11} /></span>{/if}
      </span>
    </div>
    {#if error}<div class="error" role="alert">{error}</div>{/if}
    {#if call}
      <div class="controls">
        {#if call.phase !== 'outgoing'}
          <button class="ctl" class:on={call.muted} disabled={callStore.busy} onclick={() => callStore.toggleMute()}
            title={$t(call.muted ? 'msg_call_unmute' : 'msg_call_mute')} aria-label={$t(call.muted ? 'msg_call_unmute' : 'msg_call_mute')} aria-pressed={call.muted}>
            <CallIcon name={call.muted ? 'mic-off' : 'mic'} size={15} />
          </button>
        {/if}
        <button class="ctl" class:lit={cameraOn} disabled={callStore.busy} onclick={() => callStore.toggleCamera()}
          title={$t(cameraOn ? 'msg_call_camera_off' : 'msg_call_camera_on')} aria-label={$t(cameraOn ? 'msg_call_camera_off' : 'msg_call_camera_on')} aria-pressed={cameraOn}>
          <CallIcon name={cameraOn ? 'video' : 'video-off'} size={15} />
        </button>
        <button class="ctl" onclick={() => onchat(call.chat_id)} title={$t('msg_call_open_chat')} aria-label={$t('msg_call_open_chat')}>
          <Icon name="message-circle" size={15} />
        </button>
        <button class="ctl end" disabled={callStore.ending} onclick={() => callStore.hangUp()} title={$t('msg_call_end')} aria-label={$t('msg_call_end')}>
          <CallIcon name="hangup" size={16} />
        </button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .card {
    position: fixed; top: calc(var(--overlay-inset, 0px) + 5px); left: 50%; z-index: 72; transform: translateX(-50%);
    width: max-content; max-width: min(620px, calc(100vw - 24px));
    display: flex; align-items: center; gap: var(--sp-2);
    padding: 4px 4px 4px 5px; border-radius: var(--radius-pill);
    background: var(--surface); color: var(--text); border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-lg); cursor: grab; touch-action: none; user-select: none;
    animation: drop 240ms cubic-bezier(0.2, 0.9, 0.3, 1.15);
  }
  .card:active { cursor: grabbing; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  .card.ended { opacity: 0.92; }
  .who { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; flex: 1; padding: 0 var(--sp-1); white-space: nowrap; }
  .name { font-weight: var(--fw-bold); font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; max-width: 180px; }
  .status { display: inline-flex; align-items: center; gap: 6px; font-size: var(--fs-xs); color: var(--text-2); }
  .phase.clock { font-variant-numeric: tabular-nums; color: var(--success-text); font-weight: var(--fw-semibold); }
  .ended .phase { color: var(--text-2); }
  .via, .muted-mark {
    display: inline-flex; align-items: center; gap: 3px; padding: 1px 7px; border-radius: var(--radius-pill); font-size: var(--fs-2xs);
    background: var(--surface-2); border: 1px solid var(--border); color: var(--text-2); cursor: help;
  }
  .via.relay { background: var(--accent-tint); border-color: var(--accent-tint-border); color: var(--accent-text-2); }
  .muted-mark { padding: 2px 5px; background: var(--warn-bg); border-color: var(--warn-border); color: var(--warn-text); cursor: default; }
  .rtt { color: var(--text-3); font-size: var(--fs-2xs); font-variant-numeric: tabular-nums; }
  .error {
    position: absolute; top: calc(100% + 6px); left: 50%; transform: translateX(-50%); width: max-content; max-width: 360px;
    padding: 6px 10px; border-radius: var(--radius-sm); font-size: var(--fs-xs);
    background: var(--danger-bg); border: 1px solid var(--danger-border); color: var(--danger-text); box-shadow: var(--shadow);
  }
  .controls { display: flex; gap: 4px; flex-shrink: 0; }
  .ctl {
    width: 32px; height: 32px; border-radius: 50%; border: 1px solid var(--border); cursor: pointer;
    display: inline-flex; align-items: center; justify-content: center;
    background: var(--surface-2); color: var(--text);
    transition: background var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease);
  }
  .ctl:hover:not(:disabled) { background: var(--surface-3); }
  .ctl.on { background: var(--warn-bg); border-color: var(--warn-border); color: var(--warn-text); }
  .ctl.lit { background: var(--accent-tint); border-color: var(--accent-tint-border); color: var(--accent-text-2); }
  .ctl.end { width: 40px; border-radius: var(--radius-pill); background: var(--danger); border-color: transparent; color: #fff; }
  .ctl.end:hover:not(:disabled) { background: var(--danger); filter: brightness(1.1); }
  .ctl:disabled { opacity: 0.6; cursor: default; }
  .ctl:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  @keyframes drop {
    from { opacity: 0; transform: translate(-50%, -10px) scale(0.96); }
    to { opacity: 1; transform: translate(-50%, 0) scale(1); }
  }
</style>

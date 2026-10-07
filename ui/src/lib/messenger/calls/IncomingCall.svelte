<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A call rings on a computer: a card in the middle of the window, over
  everything, with Answer and Decline (and, for a video call, Answer
  without video). Nothing but these buttons closes it: a stray Escape or
  click must not refuse a call.

  The focus goes to the card itself, not to a button: a call comes while
  the user types, and the next space or Enter meant for the chat must not
  take the call. Tab goes round the card's buttons only; when the card
  goes, the focus goes back where it was.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import CallFace from './CallFace.svelte';
  import CallIcon from './CallIcon.svelte';
  import { callStore } from './callStore.svelte';
  import { wrapTab } from './focus';
  import { callPeer } from './peer';
  import { callErrorText } from './words';

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  const call = $derived(callStore.call?.phase === 'incoming' ? callStore.call : null);
  const peer = $derived(call ? callPeer(call) : null);
  const error = $derived(callStore.error ? callErrorText(callStore.error, tr) : '');

  let card = $state<HTMLElement | null>(null);
  /** The call the focus was taken for (once per call, not at each change of it), and where it was. */
  let focusedFor = '';
  let before: HTMLElement | null = null;
  $effect(() => {
    const id = call?.call_id ?? '';
    if (id && card && focusedFor !== id) {
      focusedFor = id;
      if (!before) before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      card.focus({ preventScroll: true });
    } else if (!id && focusedFor) {
      focusedFor = '';
      const back = before;
      before = null;
      // Back where it was, unless the user has put it somewhere since.
      const lost = !document.activeElement || document.activeElement === document.body;
      if (back?.isConnected && lost) back.focus({ preventScroll: true });
    }
  });

  function keydown(e: KeyboardEvent) {
    if (!call || !card || e.key !== 'Tab') return;
    const buttons = [...card.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
    const next = wrapTab(buttons.indexOf(document.activeElement as HTMLButtonElement), buttons.length, e.shiftKey);
    if (next === null) return;
    e.preventDefault();
    if (next >= 0) buttons[next].focus();
  }
</script>

<svelte:window onkeydown={keydown} />

{#if call && peer}
  <div class="backdrop">
    <div class="card" bind:this={card} tabindex="-1" role="alertdialog" aria-modal="true" aria-labelledby="incoming-name" aria-describedby="incoming-what">
      <div class="face"><CallFace {peer} size={96} ringing /></div>
      <div class="name" id="incoming-name">{peer.name}</div>
      <div class="what" id="incoming-what">
        <CallIcon name={call.media === 'video' ? 'video' : 'incoming'} size={14} />
        {$t(call.media === 'video' ? 'msg_call_phase_incoming_video' : 'msg_call_phase_incoming')}
      </div>
      {#if error}<div class="error">{error}</div>{/if}
      <div class="actions">
        <div class="action">
          <button class="round decline" disabled={callStore.ending} onclick={() => callStore.decline()} aria-label={$t('msg_call_decline')}>
            <CallIcon name="hangup" size={26} />
          </button>
          <span>{$t('msg_call_decline')}</span>
        </div>
        <div class="action">
          <button class="round answer" disabled={callStore.busy || callStore.ending} onclick={() => callStore.accept()} aria-label={$t('msg_call_answer')}>
            <CallIcon name={call.media === 'video' ? 'video' : 'phone'} size={26} />
          </button>
          <span>{$t('msg_call_answer')}</span>
        </div>
      </div>
      {#if call.media === 'video'}
        <button class="voice" disabled={callStore.busy || callStore.ending} onclick={() => callStore.acceptWithoutVideo()}>
          <CallIcon name="video-off" size={14} />{$t('msg_call_answer_audio')}
        </button>
      {/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed; inset: var(--overlay-inset, 0); border-radius: var(--overlay-radius, 0); z-index: 80; display: flex; align-items: center; justify-content: center;
    background: var(--backdrop); animation: fade 180ms ease-out;
  }
  .card {
    width: min(340px, calc(100vw - 32px)); display: flex; flex-direction: column; align-items: center; gap: var(--sp-2);
    padding: var(--sp-10) var(--sp-6) var(--sp-6); border-radius: var(--radius-lg); overflow: hidden;
    background: var(--surface); border: 1px solid var(--border); box-shadow: var(--shadow-lg);
    animation: rise 260ms cubic-bezier(0.2, 0.9, 0.3, 1.15);
  }
  /* The card holds the focus only so that no button does: nothing to show. */
  .card:focus { outline: none; }
  .face { margin-bottom: var(--sp-3); }
  .name { font-size: var(--fs-xl); font-weight: var(--fw-extrabold); letter-spacing: -0.3px; text-align: center; overflow-wrap: anywhere; }
  .what { display: inline-flex; align-items: center; gap: 6px; font-size: var(--fs-sm); color: var(--text-2); }
  .error { font-size: var(--fs-xs); color: var(--danger-text); text-align: center; }
  .actions { display: flex; justify-content: center; gap: var(--sp-12); margin-top: var(--sp-5); }
  .action { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); font-size: var(--fs-xs); color: var(--text-2); }
  .round {
    width: 60px; height: 60px; border-radius: 50%; border: none; cursor: pointer; color: #fff;
    display: inline-flex; align-items: center; justify-content: center;
    transition: transform var(--dur-fast) var(--ease), filter var(--dur-fast) var(--ease);
  }
  .voice {
    margin-top: var(--sp-3); display: inline-flex; align-items: center; gap: 6px; padding: 7px 14px; border-radius: var(--radius-pill);
    border: 1px solid var(--border); background: var(--surface-2); color: var(--text); font: inherit; font-size: var(--fs-xs); cursor: pointer;
  }
  @media (hover: hover) { .voice:hover:not(:disabled) { background: var(--surface-3); } }
  .voice:disabled { opacity: 0.6; cursor: default; }
  .voice:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .round:hover:not(:disabled) { transform: scale(1.06); filter: brightness(1.08); }
  .round:disabled { opacity: 0.6; cursor: default; }
  .round:focus-visible { outline: 3px solid var(--accent); outline-offset: 3px; }
  .decline { background: var(--danger); box-shadow: var(--shadow-danger); }
  .answer { background: var(--success); animation: nudge 1.6s ease-in-out infinite; }
  @keyframes fade { from { opacity: 0; } }
  @keyframes rise { from { opacity: 0; transform: translateY(12px) scale(0.97); } }
  @keyframes nudge { 0%, 70%, 100% { transform: rotate(0); } 76% { transform: rotate(-14deg); } 82% { transform: rotate(12deg); } 88% { transform: rotate(-8deg); } 94% { transform: rotate(5deg); } }
  @media (prefers-reduced-motion: reduce) { .answer { animation: none; } }
</style>

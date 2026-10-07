<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A call in the chat: which way it went, how long it was or why it did not
  happen, that it went through a relay, when. A call of the peer's that I
  did not take stands out. Pressed, it calls the peer back when a call can
  be made; a refusal shows under the line for a moment, as under the call
  button of the chat's header.
-->
<script lang="ts">
  import { locale, localeTag, t } from '$lib/core/i18n';
  import type { MessengerMessage } from '../api';
  import { chatStore } from '../chats/chatStore.svelte';
  import { clock as timeOfDay } from '../shared/time';
  import { messengerStore } from '../store.svelte';
  import CallIcon from './CallIcon.svelte';
  import { callStore } from './callStore.svelte';
  import { callErrorText, callLineOf, callLineWords, duration, isMissed } from './words';

  let { message }: { message: MessengerMessage } = $props();

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  const line = $derived(callLineOf(message));
  const live = $derived(!!line && callStore.call?.call_id === line.callId);
  const missed = $derived(!!line && isMissed(line));
  const words = $derived(line ? callLineWords(line, tr, live, (s) => duration(s, localeTag($locale))) : null);
  const chat = $derived(chatStore.chats.find((c) => c.id === message.chat_id) ?? null);
  const sessionActive = $derived(!!messengerStore.status?.runtime?.session_active);
  const callable = $derived(!live && !!chat?.peer_pubkey && chat.mode === 'full_chat' && callStore.canCall() && !callStore.busy);
  const canCallBack = $derived(callable && sessionActive);
  const glyph = $derived(!line ? 'phone' : line.media === 'video' ? 'video' : missed ? 'missed' : line.direction === 'in' ? 'incoming' : 'outgoing');

  let refusal = $state('');
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function callBack() {
    if (!canCallBack || !chat?.peer_pubkey) return;
    const { view, refusal: why } = await callStore.dial(chat.peer_pubkey, line?.media ?? 'audio');
    if (view || !why) return;
    refusal = callErrorText(why, tr);
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => (refusal = ''), 4000);
  }

  $effect(() => () => { if (timer) clearTimeout(timer); });
</script>

{#if line && words}
  <div class="row" data-mid={message.id}>
    <button class="call" class:missed class:live disabled={!canCallBack} onclick={callBack}
      title={canCallBack ? $t('msg_call_back') : callable ? $t('msg_call_err_locked') : undefined}>
      <span class="ico"><CallIcon name={glyph} size={13} /></span>
      <span class="title">{words.title}</span>
      {#if words.detail}<span class="detail">{words.detail}</span>{/if}
      <span class="time">{timeOfDay(line.startedAt)}</span>
    </button>
    {#if refusal}<span class="refusal" role="status">{refusal}</span>{/if}
  </div>
{/if}

<style>
  .row { display: flex; flex-direction: column; align-items: center; gap: 4px; margin: var(--sp-2) var(--sp-4); }
  .refusal {
    max-width: 320px; padding: 4px 10px; border-radius: var(--radius-sm); font-size: var(--fs-xs); line-height: 1.4; text-align: center;
    background: var(--danger-bg); border: 1px solid var(--danger-border); color: var(--danger-text);
  }
  .call {
    display: inline-flex; align-items: center; gap: 7px; max-width: 100%;
    padding: 4px 12px 4px 4px; border-radius: var(--radius-pill);
    border: 1px solid var(--border); background: var(--surface); color: var(--text-2);
    font: inherit; font-size: var(--fs-xs); line-height: 1.3; text-align: left;
  }
  .call:not(:disabled) { cursor: pointer; }
  /* A finger leaves :hover behind after a tap: only a pointer that hovers lights it. */
  @media (hover: hover) { .call:not(:disabled):hover { border-color: var(--success-border); background: var(--success-bg); } }
  .call:disabled { opacity: 1; cursor: default; }
  .ico {
    width: 24px; height: 24px; border-radius: 50%; flex-shrink: 0;
    display: inline-flex; align-items: center; justify-content: center;
    background: var(--surface-3); color: var(--text-2);
  }
  .title { color: var(--text); font-weight: var(--fw-semibold); white-space: nowrap; }
  .detail { color: var(--text-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; font-variant-numeric: tabular-nums; }
  .time { color: var(--text-3); font-size: var(--fs-2xs); font-variant-numeric: tabular-nums; white-space: nowrap; }
  /* A call I did not take: seen from afar. */
  .missed { border-color: var(--danger-border); background: var(--danger-bg); }
  .missed .ico { background: var(--danger); color: #fff; }
  .missed .title { color: var(--danger-text); }
  .live { border-color: var(--success-border); }
  .live .ico { background: var(--success); color: #fff; }
  @media (pointer: coarse) { .call { padding-block: 6px; } }
</style>

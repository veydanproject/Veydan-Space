<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A call in the chat: which way it went, how long it was or why it did not
  happen, that it went through a relay, when. A call of the peer's that I
  did not take stands out. Pressed, it calls the peer back when a call can
  be made; a refusal shows under the line for a moment, as under the call
  button of the chat's header.

  A call of a group: who started it, that it is on now (pressed, it joins;
  while I am in it, it opens the room), how long it was and how many were
  in it. A group call over is not called back from its line: a new one
  starts from the header.
-->
<script lang="ts">
  import { countKey, locale, localeTag, t } from '$lib/core/i18n';
  import type { MessengerMessage } from '../api';
  import { chatStore } from '../chats/chatStore.svelte';
  import { nameStore } from '../groups/names.svelte';
  import { onPhone } from '../shared/phone';
  import { clock as timeOfDay } from '../shared/time';
  import { messengerStore } from '../store.svelte';
  import CallIcon from './CallIcon.svelte';
  import { callStore } from './callStore.svelte';
  import { groupCallStore } from './groupCallStore.svelte';
  import { openGroupRoom } from './room';
  import { callErrorText, callLineOf, callLineWords, duration, groupLineWords, isMissed } from './words';

  let { message }: { message: MessengerMessage } = $props();

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);
  const phone = onPhone();

  const line = $derived(callLineOf(message));
  const group = $derived(line?.kind === 'group');
  const groupId = $derived(message.chat_id.startsWith('group:') ? message.chat_id.slice('group:'.length) : '');
  /** I am in this very call (of either kind). */
  const mine = $derived(!!line && (callStore.call?.call_id === line.callId || groupCallStore.call?.call_id === line.callId));
  /** A group call on in the group now, that I am not in. */
  const on = $derived(!!line && group && !mine && groupCallStore.announcedIn(groupId)?.call_id === line.callId);
  const live = $derived(mine || on);
  const missed = $derived(!!line && isMissed(line));
  const howLong = (s: number) => duration(s, localeTag($locale));
  const words = $derived.by(() => {
    if (!line) return null;
    if (!group) return callLineWords(line, tr, live, howLong);
    const starter = line.direction === 'out' || !line.startedBy ? null : nameStore.label(line.startedBy);
    const people = (n: number) => $t(countKey('msg_gcall_people', n, $locale), { n: String(n) });
    return groupLineWords(line, tr, { live, starter, people, howLong });
  });
  const chat = $derived(chatStore.chats.find((c) => c.id === message.chat_id) ?? null);
  const sessionActive = $derived(!!messengerStore.status?.runtime?.session_active);
  const callable = $derived(!group && !live && !!chat?.peer_pubkey && chat.mode === 'full_chat' && callStore.canCall() && !callStore.busy);
  /** A group call on now: joined from its line, as from the banner. */
  const joinable = $derived(on && callStore.available && callStore.canCall() && !groupCallStore.busy);
  const canPress = $derived(group ? (mine && !!groupCallStore.call) || (joinable && sessionActive) : callable && sessionActive);
  const glyph = $derived(!line ? 'phone' : line.media === 'video' ? 'video' : group ? 'phone' : missed ? 'missed' : line.direction === 'in' ? 'incoming' : 'outgoing');
  const hint = $derived(
    group
      ? (mine ? $t('msg_call_return') : joinable ? (sessionActive ? $t('msg_gcall_join_title') : $t('msg_call_err_locked')) : undefined)
      : (canPress ? $t('msg_call_back') : callable ? $t('msg_call_err_locked') : undefined),
  );

  let refusal = $state('');
  let timer: ReturnType<typeof setTimeout> | null = null;

  function refuse(why: unknown) {
    refusal = callErrorText(why, tr);
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => (refusal = ''), 4000);
  }

  async function press() {
    if (!canPress || !line) return;
    if (group) {
      if (mine) { openGroupRoom(phone); return; }
      const { view, refusal: why } = await groupCallStore.join(groupId);
      if (view) openGroupRoom(phone);
      else if (why) refuse(why);
      return;
    }
    if (!chat?.peer_pubkey) return;
    const { view, refusal: why } = await callStore.dial(chat.peer_pubkey, line.media);
    if (!view && why) refuse(why);
  }

  $effect(() => () => { if (timer) clearTimeout(timer); });
</script>

{#if line && words}
  <div class="row" data-mid={message.id}>
    <button class="call" class:missed class:live class:group disabled={!canPress} onclick={press} title={hint}>
      <span class="ico"><CallIcon name={glyph} size={13} /></span>
      <span class="title">{words.title}</span>
      {#if words.detail}<span class="detail">{words.detail}</span>{/if}
      {#if on && canPress}<span class="join">{$t('msg_gcall_join')}</span>{/if}
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
  .join {
    padding: 1px 8px; border-radius: var(--radius-pill); background: var(--success); color: #fff;
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); white-space: nowrap;
  }
  @media (pointer: coarse) { .call { padding-block: 6px; } }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The call buttons in the header of a group, for a member: "Video call"
  and "Call" start a call of the group (a room on a call node; the members
  see it in the chat and may come in, nobody's phone rings). While a call
  is on in the group, one button joins it; while I am in it, the button
  goes back to the room. They wait while another call is under way.

  As in a conversation of two, a press asks first unless that was turned
  off (CallConfirm, kept on this device), and a hold (or a right click)
  opens the settings of calls. A refusal shows under the buttons for a
  moment.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import type { CallMedia, MessengerChat, MessengerGroup } from '../api';
  import { messengerStore } from '../store.svelte';
  import { openSettings } from '../pages/settingsTab.svelte';
  import { onPhone } from '../shared/phone';
  import CallConfirm from './CallConfirm.svelte';
  import CallIcon from './CallIcon.svelte';
  import { callStore } from './callStore.svelte';
  import { groupCallStore } from './groupCallStore.svelte';
  import { hold } from './hold';
  import { openGroupRoom } from './room';
  import { callErrorText } from './words';

  let { chat, group }: { chat: MessengerChat; group: MessengerGroup | null } = $props();

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);
  const phone = onPhone();
  /** The kinds of call the header starts, a video one first. */
  const kinds: CallMedia[] = ['video', 'audio'];

  const groupId = $derived(chat.id.slice('group:'.length));
  const member = $derived(group?.membership === 'joined');
  const mine = $derived(groupCallStore.inGroup(groupId));
  const live = $derived(mine ? null : groupCallStore.announcedIn(groupId));
  const sessionActive = $derived(!!messengerStore.status?.runtime?.session_active);
  const reason = $derived(
    callStore.loaded && !callStore.available ? $t('msg_call_unavailable')
      : callStore.call || (groupCallStore.call && !mine) ? $t('msg_call_err_busy')
      : !sessionActive ? $t('msg_call_err_locked')
      : '',
  );
  const face = $derived({ name: group?.name || chat.title, picture: group?.picture || chat.picture || null, seed: groupId });
  const name = $derived(face.name);

  let refusal = $state('');
  let timer: ReturnType<typeof setTimeout> | null = null;
  /** The kind the question is about while it is asked; `join` asks about the call on now. */
  let asking = $state<CallMedia | null>(null);
  let joining = $state(false);

  function press(media: CallMedia) {
    joining = false;
    if (callStore.confirm) asking = media;
    else void go(media);
  }

  function pressJoin() {
    if (!live) return;
    joining = true;
    if (callStore.confirm) asking = live.media;
    else void go(live.media);
  }

  async function go(media: CallMedia) {
    const { view, refusal: why } = joining ? await groupCallStore.join(groupId) : await groupCallStore.start(groupId, media);
    if (view) { openGroupRoom(phone); return; }
    if (!why) return;
    refusal = callErrorText(why, tr);
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => (refusal = ''), 4000);
  }

  // The question belongs to the chat it was asked in.
  let last = '';
  $effect(() => {
    if (chat.id !== last) { last = chat.id; asking = null; }
  });

  $effect(() => () => { if (timer) clearTimeout(timer); });

  const toSettings = { onhold: () => { asking = null; openSettings('calls', phone); } };
  const question = $derived(
    joining ? $t('msg_gcall_confirm_join', { name })
      : $t(asking === 'video' ? 'msg_gcall_confirm_video' : 'msg_gcall_confirm_audio', { name }),
  );
</script>

{#if member}
  <span class="wrap">
    {#if mine}
      <button class="pill on" class:round={phone} onclick={() => openGroupRoom(phone)} title={$t('msg_call_return')} aria-label={$t('msg_call_return')}>
        <CallIcon name="phone" size={14} />{#if !phone}<span class="label">{$t('msg_gcall_in_room')}</span>{/if}
      </button>
    {:else if live}
      <button class="pill" class:round={phone} disabled={!!reason || groupCallStore.busy} onclick={pressJoin} use:hold={toSettings}
        title={reason || $t('msg_gcall_join_title')} aria-label={$t('msg_gcall_join_title')}>
        <CallIcon name={live.media === 'video' ? 'video' : 'phone'} size={14} />{#if !phone}<span class="label">{$t('msg_gcall_join')}</span>{/if}
      </button>
    {:else}
      {#if kinds.includes('video')}
        <button class="icon" disabled={!!reason || groupCallStore.busy} onclick={() => press('video')} use:hold={toSettings}
          title={reason || $t('msg_gcall_video_start')} aria-label={$t('msg_gcall_video_start')}>
          <CallIcon name="video" size={17} />
        </button>
      {/if}
      <button class="icon" disabled={!!reason || groupCallStore.busy} onclick={() => press('audio')} use:hold={toSettings}
        title={reason || $t('msg_gcall_start')} aria-label={$t('msg_gcall_start')}>
        <CallIcon name="phone" size={16} />
      </button>
    {/if}
    {#if refusal}<span class="refusal" role="status">{refusal}</span>{/if}
  </span>
  <CallConfirm media={asking} peer={face} {question} action={joining ? $t('msg_gcall_join') : undefined}
    oncall={(m) => void go(m)} onclose={() => (asking = null)} />
{/if}

<style>
  .wrap { position: relative; display: inline-flex; align-items: center; }
  .icon {
    border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm);
    -webkit-touch-callout: none; user-select: none; -webkit-user-select: none;
  }
  @media (hover: hover) { .icon:hover:not(:disabled) { color: var(--success-text); background: var(--success-bg); } }
  .icon:disabled, .pill:disabled { opacity: 0.45; cursor: default; }
  .pill {
    display: inline-flex; align-items: center; gap: 5px; margin: 0 4px; padding: 4px 10px 4px 8px; border-radius: var(--radius-pill);
    border: none; cursor: pointer; font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-bold);
    background: var(--success); color: #fff; -webkit-touch-callout: none; user-select: none; -webkit-user-select: none;
  }
  .pill.on { background: var(--success-bg); color: var(--success-text); border: 1px solid var(--success-border); }
  @media (hover: hover) { .pill:hover:not(:disabled) { filter: brightness(1.08); } }
  .label { white-space: nowrap; }
  @media (pointer: coarse) { .icon { padding: 10px; } .pill { padding: 6px 12px 6px 10px; } }
  /* A phone: the pill is a round button, its words are the banner's. */
  .pill.round { padding: 8px; }
  .refusal {
    position: absolute; top: calc(100% + 6px); right: 0; z-index: 5; width: max-content; max-width: 260px;
    padding: 6px 10px; border-radius: var(--radius-sm); font-size: var(--fs-xs); line-height: 1.4;
    background: var(--danger-bg); border: 1px solid var(--danger-border); color: var(--danger-text); box-shadow: var(--shadow);
  }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  "Video call" and "Call" in the header of a conversation of two. Only a
  contact I am in a mutual chat with can be called (the runtime refuses
  anyone else, and a direct call shows my address to the peer), so the
  buttons are there only then; they wait while another call is under way.
  A refusal shows under the buttons for a moment.

  A press asks first "Start a voice call?" / "Start a video call?" unless
  that was turned off (CallConfirm, the setting kept on this device). A
  hold of either button (a finger or the mouse), or a right click, opens
  the settings of calls.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import type { CallMedia, MessengerChat } from '../api';
  import { messengerStore } from '../store.svelte';
  import { openSettings } from '../pages/settingsTab.svelte';
  import { onPhone } from '../shared/phone';
  import CallConfirm from './CallConfirm.svelte';
  import CallIcon from './CallIcon.svelte';
  import { callStore } from './callStore.svelte';
  import { hold } from './hold';
  import { callErrorText } from './words';

  let { chat }: { chat: MessengerChat } = $props();

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  const callable = $derived(chat.kind === 'dm' && chat.mode === 'full_chat' && !!chat.peer_pubkey);
  const sessionActive = $derived(!!messengerStore.status?.runtime?.session_active);
  const reason = $derived(
    callStore.loaded && !callStore.available ? $t('msg_call_unavailable')
      : callStore.call ? $t('msg_call_err_busy')
      : !sessionActive ? $t('msg_call_err_locked')
      : '',
  );
  const peer = $derived({ name: chat.title, picture: chat.picture ?? null, seed: chat.peer_pubkey ?? chat.id });

  let refusal = $state('');
  let timer: ReturnType<typeof setTimeout> | null = null;
  /** The kind the question is about while it is asked. */
  let asking = $state<CallMedia | null>(null);

  function press(media: CallMedia) {
    if (callStore.confirm) asking = media;
    else void call(media);
  }

  async function call(media: CallMedia) {
    if (!chat.peer_pubkey) return;
    const { view, refusal: why } = await callStore.dial(chat.peer_pubkey, media);
    if (view || !why) return;
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

  const phone = onPhone();
  const toSettings = { onhold: () => { asking = null; openSettings('calls', phone); } };
</script>

{#if callable}
  <span class="wrap">
    <button class="icon" disabled={!!reason || callStore.busy} onclick={() => press('video')} use:hold={toSettings}
      title={reason || $t('msg_call_video_start')} aria-label={$t('msg_call_video_start')}>
      <CallIcon name="video" size={17} />
    </button>
    <button class="icon" disabled={!!reason || callStore.busy} onclick={() => press('audio')} use:hold={toSettings}
      title={reason || $t('msg_call_start')} aria-label={$t('msg_call_start')}>
      <CallIcon name="phone" size={16} />
    </button>
    {#if refusal}<span class="refusal" role="status">{refusal}</span>{/if}
  </span>
  <CallConfirm media={asking} {peer} oncall={(m) => void call(m)} onclose={() => (asking = null)} />
{/if}

<style>
  .wrap { position: relative; display: inline-flex; }
  .icon {
    border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm);
    -webkit-touch-callout: none; user-select: none; -webkit-user-select: none;
  }
  @media (hover: hover) { .icon:hover:not(:disabled) { color: var(--success-text); background: var(--success-bg); } }
  .icon:disabled { opacity: 0.45; cursor: default; }
  @media (pointer: coarse) { .icon { padding: 10px; } }
  .refusal {
    position: absolute; top: calc(100% + 6px); right: 0; z-index: 5; width: max-content; max-width: 260px;
    padding: 6px 10px; border-radius: var(--radius-sm); font-size: var(--fs-xs); line-height: 1.4;
    background: var(--danger-bg); border: 1px solid var(--danger-border); color: var(--danger-text); box-shadow: var(--shadow);
  }
</style>

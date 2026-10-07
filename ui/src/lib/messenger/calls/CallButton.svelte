<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  "Call" and "Video call" in the header of a conversation of two. Only a contact I am in a
  mutual chat with can be called (the runtime refuses anyone else, and a
  direct call shows my address to the peer), so the button is there only
  then; it waits while another call is under way. A refusal shows under
  the button for a moment.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import type { CallMedia, MessengerChat } from '../api';
  import { messengerStore } from '../store.svelte';
  import CallIcon from './CallIcon.svelte';
  import { callStore } from './callStore.svelte';
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

  let refusal = $state('');
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function call(media: CallMedia) {
    if (!chat.peer_pubkey) return;
    const { view, refusal: why } = await callStore.dial(chat.peer_pubkey, media);
    if (view || !why) return;
    refusal = callErrorText(why, tr);
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => (refusal = ''), 4000);
  }

  $effect(() => () => { if (timer) clearTimeout(timer); });
</script>

{#if callable}
  <span class="wrap">
    <button class="icon" disabled={!!reason || callStore.busy} onclick={() => call('video')} title={reason || $t('msg_call_video_start')} aria-label={$t('msg_call_video_start')}>
      <CallIcon name="video" size={17} />
    </button>
    <button class="icon" disabled={!!reason || callStore.busy} onclick={() => call('audio')} title={reason || $t('msg_call_start')} aria-label={$t('msg_call_start')}>
      <CallIcon name="phone" size={16} />
    </button>
    {#if refusal}<span class="refusal" role="status">{refusal}</span>{/if}
  </span>
{/if}

<style>
  .wrap { position: relative; display: inline-flex; }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  @media (hover: hover) { .icon:hover:not(:disabled) { color: var(--success-text); background: var(--success-bg); } }
  .icon:disabled { opacity: 0.45; cursor: default; }
  @media (pointer: coarse) { .icon { padding: 10px; } }
  .refusal {
    position: absolute; top: calc(100% + 6px); right: 0; z-index: 5; width: max-content; max-width: 260px;
    padding: 6px 10px; border-radius: var(--radius-sm); font-size: var(--fs-xs); line-height: 1.4;
    background: var(--danger-bg); border: 1px solid var(--danger-border); color: var(--danger-text); box-shadow: var(--shadow);
  }
</style>

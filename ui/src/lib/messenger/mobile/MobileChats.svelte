<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import ChatList from '../chats/ChatList.svelte';
  import NewChatDialog from '../chats/NewChatDialog.svelte';
  import NewGroupDialog from "../groups/NewGroupDialog.svelte";
  import InvitesBar from "../groups/InvitesBar.svelte";
  import BridgeOffer from "../net/BridgeOffer.svelte";
  import Onboarding from '../onboarding/Onboarding.svelte';
  import MobileFrame from './MobileFrame.svelte';
  import { chatHref } from './routes';
  import PushOffer from '../push/PushOffer.svelte';
  import { pushSeen, startPushBridge } from '../push/bridge';
  import type { MessengerChat } from '../api';
  import { markPhone } from '../shared/phone';

  markPhone();

  let query = $state('');
  let newChat = $state(false);
  let newGroup = $state(false);

  onMount(() => {
    chatStore.close();
    messengerStore.refresh().catch(() => {});
    messengerStore.startListeners().catch(() => {});
    // The list shows what is new: the notification about direct messages
    // has done its work.
    startPushBridge().then(() => pushSeen()).catch(() => {});
  });

  const s = $derived(messengerStore.status);
  const connected = $derived(s?.runtime?.relays_connected ?? 0);
  const total = $derived(s?.runtime?.relays_total ?? 0);
  const netState = $derived(
    !s?.runtime ? 'off' : s.runtime.silent_mode ? 'silent' : !s.runtime.session_active ? 'locked' : connected > 0 ? 'on' : 'connecting',
  );
  const needsIdentity = $derived(messengerStore.needsOnboarding);

  const open = (c: MessengerChat) => goto(chatHref(c.id));
</script>

{#snippet lead()}
  <span class="alpha">{$t('msg_alpha_badge')}</span>
  <span class="net {netState}" title={$t(`msg_net_${netState}` as 'msg_net_on', { connected: String(connected), total: String(total) })}></span>
{/snippet}

{#snippet actions()}
  {#if !needsIdentity && s?.runtime}
    <button class="ibtn" onclick={() => goto('/messenger/settings')} aria-label={$t('msg_settings_title_full')}><Icon name="user-cog" size={21} /></button>
    <button class="ibtn" onclick={() => goto('/messenger/contacts')} aria-label={$t('msg_contacts_title')}><Icon name="book-user" size={21} /></button>
    <button class="ibtn" onclick={() => (newGroup = true)} aria-label={$t("msg_group_new_title")}><Icon name="users-plus" size={21} /></button>
  {/if}
{/snippet}

<MobileFrame title={$t('msg_title')} {lead} {actions} scroll={needsIdentity || !s?.runtime}>
  {#if !s}
    <div class="note">{$t('loading')}</div>
  {:else if !s.compiled}
    <div class="error-msg">{$t('msg_status_not_compiled')}</div>
  {:else if s.error}
    <div class="error-msg">{$t('msg_status_error', { error: s.error })}</div>
  {:else if !s.enabled}
    <div class="note">{$t('msg_mobile_disabled')}</div>
  {:else if needsIdentity}
    <p class="intro">{$t('msg_intro')}</p>
    <Onboarding />
  {:else}
    {#if netState === 'locked' || netState === 'silent'}
      <div class="strip">{$t(`msg_net_${netState}` as 'msg_net_locked', { connected: '0', total: '0' })}</div>
    {/if}
    <div class="search">
      <Icon name="search" size={16} />
      <input type="search" bind:value={query} placeholder={$t('msg_chats_search')} spellcheck="false" enterkeyhint="search" />
      {#if query}<button class="clear" onclick={() => (query = '')} aria-label={$t('msg_back')}><Icon name="x" size={14} /></button>{/if}
    </div>
    <BridgeOffer />
    <PushOffer />
    <InvitesBar />
    <ChatList {query} onopen={open} />
    <button class="fab" onclick={() => (newChat = true)} aria-label={$t('msg_newchat_title')}><Icon name="message-circle-plus" size={22} /></button>
  {/if}
</MobileFrame>

<NewChatDialog bind:open={newChat} onopened={() => { if (chatStore.activeId) goto(chatHref(chatStore.activeId)); }} />
<NewGroupDialog bind:open={newGroup} onopened={(id) => goto(chatHref(id))} />

<style>
  .alpha {
    margin-left: 8px; font-size: 10px; font-weight: var(--fw-bold); text-transform: uppercase; letter-spacing: 0.6px;
    padding: 2px 7px; border-radius: var(--radius-sm); background: var(--accent-tint); color: var(--accent-text-2);
  }
  .net { width: 9px; height: 9px; border-radius: 50%; margin-left: 8px; background: var(--text-3); flex-shrink: 0; }
  .net.on { background: var(--success); box-shadow: 0 0 0 3px var(--success-bg); }
  .net.connecting { background: var(--warn-text); animation: pulse 1.4s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.35; } }
  .note { padding: var(--sp-6) var(--sp-4); text-align: center; color: var(--text-2); font-size: var(--fs-sm); line-height: 1.5; }
  .intro { margin: 0; color: var(--text-2); font-size: var(--fs-sm); line-height: 1.5; }
  .strip { padding: var(--sp-2) var(--sp-4); font-size: var(--fs-xs); color: var(--warn-text); background: var(--warn-bg); border-bottom: 1px solid var(--warn-border); }
  .search {
    display: flex; align-items: center; gap: 8px; margin: var(--sp-2) var(--sp-3); padding: 0 12px; flex-shrink: 0;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-pill); color: var(--text-3);
  }
  /* The pill is the field: its border is the one focus mark. */
  .search:focus-within { border-color: var(--accent-border); }
  .search input { flex: 1; min-width: 0; border: none; border-radius: 0; background: none; outline: none; box-shadow: none; font: inherit; font-size: 16px; color: var(--text); min-height: 42px; padding: 0; }
  .search input::-webkit-search-cancel-button { display: none; }
  .clear { border: none; background: none; color: var(--text-3); display: inline-flex; padding: 8px; }
  .fab {
    position: absolute; right: var(--sp-4); bottom: var(--sp-4); width: 56px; height: 56px; border: none; border-radius: 50%;
    background: var(--accent-grad); color: #fff; display: inline-flex; align-items: center; justify-content: center;
    box-shadow: var(--shadow-accent);
  }
  .fab:active { filter: brightness(0.92); }
</style>

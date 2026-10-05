<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import '../shared/emoji/font.css';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { netState as netStateOf } from '../shared/net-state';
  import { chatStore } from '../chats/chatStore.svelte';
  import Onboarding from '../onboarding/Onboarding.svelte';
  import ChatList from '../chats/ChatList.svelte';
  import NewChatDialog from '../chats/NewChatDialog.svelte';
  import DmChat from '../dm/DmChat.svelte';
  import GroupChat from "../groups/GroupChat.svelte";
  import NewGroupDialog from "../groups/NewGroupDialog.svelte";
  import InvitesBar from "../groups/InvitesBar.svelte";
  import BridgeOffer from "../net/BridgeOffer.svelte";
  import ContactsPanel from '../contacts/ContactsPanel.svelte';
  import SettingsView from './SettingsView.svelte';
  import ConfirmHost from '../shared/ConfirmHost.svelte';
  import type { MessengerChat } from '../api';
  import { onChatOpened } from '../content/actions';
  import { paletteUi } from '../palette.svelte';

  type View = 'chat' | 'contacts' | 'settings';
  let view = $state<View>('chat');
  let query = $state('');
  let newChat = $state(false);
  let newGroup = $state(false);

  onMount(() => {
    messengerStore.refresh().catch(() => {});
    messengerStore.startListeners().catch(() => {});
  });
  // A chat opened from elsewhere (a link, a notification) is shown here.
  onMount(() => onChatOpened(() => { view = 'chat'; }));

  const s = $derived(messengerStore.status);
  const connected = $derived(s?.runtime?.relays_connected ?? 0);
  const total = $derived(s?.runtime?.relays_total ?? 0);
  const netState = $derived(netStateOf(s?.runtime));

  function openChat(c: MessengerChat) {
    view = 'chat';
    chatStore.open(c.id).catch(() => {});
  }

  // A command of the palette asked for a dialog or a pane.
  $effect(() => {
    const r = paletteUi.request;
    if (!r) return;
    paletteUi.request = null;
    if (r === 'newChat') newChat = true;
    else if (r === 'newGroup') newGroup = true;
    else show(r);
  });

  // The identity was removed from Settings: after the onboarding the chats come back, not that pane.
  $effect(() => {
    if (!messengerStore.identity && view === 'settings') view = 'chat';
  });

  function show(v: View) {
    view = v;
    if (v !== 'chat') chatStore.close();
  }
</script>

{#if !s}
  <div class="page"><div class="empty-state">{$t('loading')}</div></div>
{:else if !s.compiled}
  <div class="page"><div class="error-msg">{$t('msg_status_not_compiled')}</div></div>
{:else if s.error}
  <div class="page"><div class="error-msg">{$t('msg_status_error', { error: s.error })}</div></div>
{:else if messengerStore.needsOnboarding}
  <div class="page">
    <div class="page-header">
      <h1>{$t('msg_title')} <span class="alpha-badge">{$t('msg_alpha_badge')}</span></h1>
    </div>
    <p class="page-sub">{$t('msg_intro')}</p>
    <Onboarding />
  </div>
{:else if s.runtime}
  <div class="page page--fill messenger">
    <div class="shell">
      <aside class="rail" class:hidden-narrow={view !== 'chat' || !!chatStore.active}>
        <div class="rail-head">
          <span class="rail-title">{$t('msg_title')} <span class="alpha-badge">{$t('msg_alpha_badge')}</span></span>
          <span class="net {netState}" title={$t(`msg_net_${netState}` as 'msg_net_on', { connected: String(connected), total: String(total) })}></span>
          <span class="spacer"></span>
          <button class="icon" class:active={view === 'settings'} onclick={() => show(view === 'settings' ? 'chat' : 'settings')} title={$t('msg_settings_title_full')}><Icon name="user-cog" size={16} /></button>
          <button class="icon" class:active={view === 'contacts'} onclick={() => show(view === 'contacts' ? 'chat' : 'contacts')} title={$t('msg_contacts_title')}><Icon name="book-user" size={16} /></button>
          <button class="icon" onclick={() => (newGroup = true)} title={$t("msg_group_new_title")}><Icon name="users-plus" size={16} /></button>
          <button class="icon accent" onclick={() => (newChat = true)} title={$t('msg_newchat_title')}><Icon name="message-circle-plus" size={16} /></button>
        </div>
        <div class="search">
          <Icon name="search" size={14} />
          <input type="text" bind:value={query} placeholder={$t('msg_chats_search')} spellcheck="false" />
          {#if query}<button class="icon sm" onclick={() => (query = '')}><Icon name="x" size={12} /></button>{/if}
        </div>
        <BridgeOffer />
        <InvitesBar />
        <ChatList {query} onopen={openChat} />
      </aside>

      <main class="pane" class:hidden-narrow={view === 'chat' && !chatStore.active}>
        {#if view === 'contacts'}
          <div class="pane-scroll">
            <button class="back-narrow btn btn-ghost btn-sm" onclick={() => show('chat')}><Icon name="arrow-left" size={12} />{$t('msg_back')}</button>
            <ContactsPanel onchat={(pubkey) => { view = 'chat'; chatStore.openPeer(pubkey).catch(() => {}); }} />
          </div>
        {:else if view === 'settings'}
          <div class="pane-scroll">
            <button class="back-narrow btn btn-ghost btn-sm" onclick={() => show('chat')}><Icon name="arrow-left" size={12} />{$t('msg_back')}</button>
            <SettingsView />
          </div>
        {:else if chatStore.active?.kind === "group"}
          <GroupChat chat={chatStore.active} onback={() => chatStore.close()} />
        {:else if chatStore.active}
          <DmChat chat={chatStore.active} onback={() => chatStore.close()} />
        {:else}
          <div class="welcome">
            <div class="welcome-icon"><Icon name="message-circle" size={34} /></div>
            <p>{$t('msg_welcome_title')}</p>
            <span>{$t('msg_welcome_text')}</span>
            <div class="welcome-actions">
                <button class="btn btn-primary" onclick={() => (newChat = true)}><Icon name="message-circle-plus" size={14} />{$t('msg_newchat_title')}</button>
              <button class="btn btn-ghost" onclick={() => (newGroup = true)}><Icon name="users-plus" size={14} />{$t("msg_group_new_title")}</button>
            </div>
          </div>
        {/if}
      </main>
    </div>
  </div>
  <NewChatDialog bind:open={newChat} onopened={() => (view = 'chat')} />
  <NewGroupDialog bind:open={newGroup} onopened={() => (view = "chat")} />
  <ConfirmHost />
{/if}

<style>
  .alpha-badge {
    display: inline-block; vertical-align: middle; margin-left: 6px;
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); text-transform: uppercase; letter-spacing: 0.6px;
    padding: 2px 7px; border-radius: var(--radius-sm);
    background: var(--accent-tint); color: var(--accent-text-2);
  }
  .messenger { max-width: none; }
  .shell {
    flex: 1; min-height: 0; display: grid; grid-template-columns: 340px minmax(0, 1fr);
    border: 1px solid var(--border); border-radius: var(--radius-lg); overflow: hidden; background: var(--surface);
  }
  .rail { display: flex; flex-direction: column; min-height: 0; border-right: 1px solid var(--border); background: var(--surface); }
  .rail-head { display: flex; align-items: center; gap: 2px; padding: var(--sp-3) var(--sp-3) var(--sp-2) var(--sp-4); min-height: 56px; }
  /* One line in every language: the badge stays beside the name. */
  .rail-title { display: inline-flex; align-items: center; white-space: nowrap; flex-shrink: 0; font-weight: var(--fw-extrabold); font-size: var(--fs-md); letter-spacing: -0.3px; }
  .spacer { flex: 1; }
  .net { width: 8px; height: 8px; border-radius: 50%; margin-left: 8px; background: var(--text-3); flex-shrink: 0; }
  .net.on { background: var(--success); box-shadow: 0 0 0 3px var(--success-bg); }
  .net.connecting { background: var(--warn-text); animation: pulse 1.4s ease-in-out infinite; }
  .net.lost { background: var(--danger); }
  .net.silent, .net.locked { background: var(--text-3); }
  @keyframes pulse { 50% { opacity: 0.35; } }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 7px; border-radius: var(--radius-sm); }
  .icon:hover { color: var(--text); background: var(--surface-3); }
  .icon.active { color: var(--accent-text-2); background: var(--accent-tint); }
  .icon.accent { color: var(--accent-text-2); }
  .icon.sm { padding: 3px; }
  .search {
    display: flex; align-items: center; gap: 8px; margin: 0 var(--sp-3) var(--sp-2); padding: 0 10px;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-pill); color: var(--text-3);
  }
  .search:focus-within { border-color: var(--accent-border); }
  /* The pill is the field: its border is the one focus mark. */
  .search input { flex: 1; min-width: 0; border: none; border-radius: 0; background: none; outline: none; box-shadow: none; font: inherit; font-size: var(--fs-sm); color: var(--text); padding: 7px 0; }
  .pane { min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg); }
  .pane-scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-5); display: flex; flex-direction: column; gap: var(--sp-4); }
  .back-narrow { display: none; align-self: flex-start; }
  .welcome { margin: auto; display: flex; flex-direction: column; align-items: center; gap: var(--sp-3); text-align: center; padding: var(--sp-6); color: var(--text-3); }
  .welcome-icon { width: 72px; height: 72px; border-radius: 50%; background: var(--accent-tint); color: var(--accent-text-2); display: flex; align-items: center; justify-content: center; }
  .welcome p { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); }
  .welcome span { font-size: var(--fs-sm); max-width: 360px; line-height: 1.5; color: var(--text-2); }
  .welcome-actions { display: flex; gap: var(--sp-2); flex-wrap: wrap; justify-content: center; }

  @media (max-width: 860px) {
    .shell { grid-template-columns: minmax(0, 1fr); }
    .hidden-narrow { display: none; }
    .back-narrow { display: inline-flex; }
  }
</style>

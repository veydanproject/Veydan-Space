<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Skeleton from '$lib/core/ui/Skeleton.svelte';
  import { messengerStore } from '../store.svelte';
  import { netState as netStateOf } from '../shared/net-state';
  import { chatStore } from '../chats/chatStore.svelte';
  import ChatList from '../chats/ChatList.svelte';
  import NewChatDialog from '../chats/NewChatDialog.svelte';
  import NewGroupDialog from "../groups/NewGroupDialog.svelte";
  import InvitesBar from "../groups/InvitesBar.svelte";
  import BridgeOffer from "../net/BridgeOffer.svelte";
  import Onboarding from '../onboarding/Onboarding.svelte';
  import MobileFrame from './MobileFrame.svelte';
  import { chatHref, sectionItemHref } from './routes';
  import { chatSections } from '../seams/sections.svelte';
  import PushOffer from '../push/PushOffer.svelte';
  import { pushSeen, startPushBridge } from '../push/bridge';
  import type { MessengerChat } from '../api';
  import { markPhone } from '../shared/phone';
  import type { ChatSection } from '../chats/sections';
  import MeButton from './MeButton.svelte';

  /** The tab: the chats between two, the groups (the same list, of the groups only while nothing is typed), or a section a module gives. */
  let { kind = 'dm' }: { kind?: ChatSection | string } = $props();

  /** The section a module gives, when the tab is one (seams/sections: the channels). */
  const host = $derived(kind === 'dm' || kind === 'group' ? null : chatSections.get(kind));
  let creating = $state(false);

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
  const netState = $derived(netStateOf(s?.runtime));
  const needsIdentity = $derived(messengerStore.needsOnboarding);

  const open = (c: MessengerChat) => goto(chatHref(c.id));
</script>

{#snippet lead()}
  <span class="net {netState}" title={$t(`msg_net_${netState}` as 'msg_net_on', { connected: String(connected), total: String(total) })}></span>
{/snippet}

<!-- The offers are the head of the list: they scroll away with the rows rather than stand over them. -->
{#snippet offers()}
  <div class="offers">
    <BridgeOffer />
    <PushOffer />
  </div>
{/snippet}

{#snippet actions()}
  {#if !needsIdentity && s?.runtime}<MeButton />{/if}
{/snippet}

<MobileFrame hubButton title={host ? $t(host.title) : kind === 'group' ? $t('msg_nav_groups') : $t('msg_nav_chats')} {lead} {actions} scroll={needsIdentity || !s?.runtime}>
  {#if !s}
    <Skeleton height={64} size={44} trailing />
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
    {#if host}
      {#if host.above}{@const Above = host.above}<Above />{/if}
    {:else if kind !== 'dm'}
      <InvitesBar />
    {/if}
    <ChatList {query} {kind} onopen={open} onopenrow={(id) => goto(sectionItemHref(id))} top={!host && kind === 'dm' ? offers : undefined} />
    {#if host}
      <button class="fab" onclick={() => (creating = true)} aria-label={$t(host.newTitle)}><Icon name={host.newIcon} size={22} /></button>
    {:else if kind === 'group'}
      <button class="fab" onclick={() => (newGroup = true)} aria-label={$t('msg_groups_new')}><Icon name="users-plus" size={22} /></button>
    {:else}
      <button class="fab" onclick={() => (newChat = true)} aria-label={$t('msg_newchat_title')}><Icon name="message-circle-plus" size={22} /></button>
    {/if}
  {/if}
</MobileFrame>

<NewChatDialog bind:open={newChat} shortcuts onopened={() => { if (chatStore.activeId) goto(chatHref(chatStore.activeId)); }} />
<NewGroupDialog bind:open={newGroup} onopened={(id) => goto(chatHref(id))} />
{#if host}{@const Create = host.create}<Create bind:open={creating} onopened={(id) => goto(sectionItemHref(id))} />{/if}

<style>
  .net { width: 9px; height: 9px; border-radius: 50%; margin-left: 8px; background: var(--text-3); flex-shrink: 0; }
  .net.on { background: var(--success); box-shadow: 0 0 0 3px var(--success-bg); }
  .net.connecting { background: var(--warn-text); animation: pulse 1.4s ease-in-out infinite; }
  .net.lost { background: var(--danger); }
  @keyframes pulse { 50% { opacity: 0.35; } }
  .note { padding: var(--sp-6) var(--sp-4); text-align: center; color: var(--text-2); font-size: var(--fs-sm); line-height: 1.5; }
  .intro { margin: 0; color: var(--text-2); font-size: var(--fs-sm); line-height: 1.5; }
  .strip { padding: var(--sp-2) var(--sp-4); font-size: var(--fs-xs); color: var(--warn-text); background: var(--warn-bg); border-bottom: 1px solid var(--warn-border); }
  .search {
    display: flex; align-items: center; gap: var(--sp-2); margin: var(--sp-2) var(--sp-3); padding: 0 var(--sp-4); flex-shrink: 0;
    background: var(--surface-3); border: 1px solid transparent; border-radius: var(--radius-pill); color: var(--text-3);
  }
  /* The pill is the field: its border is the one focus mark. */
  .search:focus-within { border-color: var(--accent-border); }
  .search input { flex: 1; min-width: 0; border: none; border-radius: 0; background: none; outline: none; box-shadow: none; font: inherit; font-size: var(--fs-md); color: var(--text); min-height: 42px; padding: 0; }
  .search input::-webkit-search-cancel-button { display: none; }
  .clear { border: none; background: none; color: var(--text-3); display: inline-flex; padding: var(--sp-2); }
  /* Inside the list, whose own side gap (--sp-2) would add to the cards' margin: they keep the edge of the search. */
  .offers { margin: 0 calc(-1 * var(--sp-2)); }
  .fab {
    position: absolute; right: var(--sp-4); bottom: var(--sp-4); width: 56px; height: 56px; border: none; border-radius: 50%;
    background: var(--accent-grad); color: #fff; display: inline-flex; align-items: center; justify-content: center;
    box-shadow: var(--shadow-accent);
  }
  .fab:active { filter: brightness(0.92); }
  /* The list scrolls its last row out from under the «+»: the button (56), its offset and the list's own gap.
     The bar of sections lies under .content and holds the safe area itself. */
  :global(.list-wrap):has(~ .fab) { padding-bottom: calc(56px + var(--sp-4) + var(--sp-2)); }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { refocusSearch } from '$lib/core/ui/searchField';
  import Notice from '$lib/core/ui/Notice.svelte';
  import Skeleton from '$lib/core/ui/Skeleton.svelte';
  import { messengerStore } from '../store.svelte';
  import { netDot, netState as netStateOf } from '../shared/net-state';
  import { chatStore } from '../chats/chatStore.svelte';
  import ChatList from '../chats/ChatList.svelte';
  import NewChatDialog from '../chats/NewChatDialog.svelte';
  import NewGroupDialog from "../groups/NewGroupDialog.svelte";
  import InvitesBar from "../groups/InvitesBar.svelte";
  import BridgeOffer from "../net/BridgeOffer.svelte";
  import { netStore } from '../net/netStore.svelte';
  import { onboarding } from '$lib/core/ui/onboarding';
  import Onboarding from '../onboarding/Onboarding.svelte';
  import IdentityOnboarding from '../identity/IdentityOnboarding.svelte';
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
  /** One first start (design 15): the account's forms are the wizard's step, opened again at it from here. */
  const wizard = $derived(onboarding.has('account'));
  /** The key is here, whose servers are not chosen yet: the wizard's step asks that. */
  const keyOnly = $derived(!!messengerStore.identity && !messengerStore.pendingBackup);

  const open = (c: MessengerChat) => goto(chatHref(c.id));
</script>

{#snippet lead()}
  <span class="status-dot {netDot(netState)}" title={$t(`msg_net_${netState}` as 'msg_net_on', { connected: String(connected), total: String(total) })}></span>
{/snippet}

<!-- The offers are the head of the list: they scroll away with the rows rather than stand over them. -->
{#snippet offers()}
  <div class="offers">
    <!-- One card at a time: the bridge first, the notifications once the bridge is answered. -->
    <BridgeOffer />
    {#if !netStore.toOffer}<PushOffer />{/if}
  </div>
{/snippet}

{#snippet actions()}
  {#if !needsIdentity && s?.runtime}<MeButton />{/if}
{/snippet}

<MobileFrame hubButton title={host ? $t(host.title) : kind === 'group' ? $t('msg_nav_groups') : $t('msg_nav_chats')} {lead} {actions} scroll={needsIdentity || !s?.runtime}>
  {#if !s}
    <Skeleton height={64} size={44} trailing />
  {:else if !s.compiled}
    <Notice tone="danger">{$t('msg_status_not_compiled')}</Notice>
  {:else if s.error}
    <Notice tone="danger">{$t('msg_status_error', { error: s.error })}</Notice>
  {:else if !s.enabled}
    <div class="note">{$t('msg_mobile_disabled')}</div>
  {:else if needsIdentity}
    <p class="intro">{$t('msg_intro')}</p>
    {#if !wizard}
      <Onboarding />
    {:else if keyOnly}
      <button type="button" class="btn btn-primary wide" onclick={() => onboarding.resume('account')}>{$t('msg_ob_continue')}</button>
    {:else}
      <IdentityOnboarding onwizard={(intent) => onboarding.resume('account', intent)} />
    {/if}
  {:else}
    {#if netState === 'locked' || netState === 'silent'}
      <div class="strip"><Notice tone="warn" role="status">{$t(`msg_net_${netState}` as 'msg_net_locked', { connected: '0', total: '0' })}</Notice></div>
    {/if}
    <div class="search-field fill list-search">
      <Icon name="search" size={16} />
      <input type="search" bind:value={query} placeholder={$t('msg_chats_search')} spellcheck="false" enterkeyhint="search" />
      {#if query}
        <button type="button" class="search-clear" onclick={(e) => { query = ''; refocusSearch(e); }} aria-label={$t('common_clear')}><Icon name="x" size={16} /></button>
      {/if}
    </div>
    {#if host}
      {#if host.above}{@const Above = host.above}<Above />{/if}
    {:else if kind !== 'dm'}
      <InvitesBar />
    {/if}
    <ChatList {query} {kind} onopen={open} onopenfound={(c, q) => goto(chatHref(c.id, q))} onopenrow={(id) => goto(sectionItemHref(id))} top={!host && kind === 'dm' ? offers : undefined} />
    {#if host}
      <button class="m-fab" onclick={() => (creating = true)} aria-label={$t(host.newTitle)}><Icon name={host.newIcon} size={22} /></button>
    {:else if kind === 'group'}
      <button class="m-fab" onclick={() => (newGroup = true)} aria-label={$t('msg_groups_new')}><Icon name="users-plus" size={22} /></button>
    {:else}
      <button class="m-fab" onclick={() => (newChat = true)} aria-label={$t('msg_newchat_title')}><Icon name="message-circle-plus" size={22} /></button>
    {/if}
  {/if}
</MobileFrame>

<NewChatDialog bind:open={newChat} shortcuts onopened={() => { if (chatStore.activeId) goto(chatHref(chatStore.activeId)); }} />
<NewGroupDialog bind:open={newGroup} onopened={(id) => goto(chatHref(id))} />
{#if host}{@const Create = host.create}<Create bind:open={creating} onopened={(id) => goto(sectionItemHref(id))} />{/if}

<style>
  .note { padding: var(--sp-6) var(--sp-4); text-align: center; color: var(--text-2); font-size: var(--fs-sm); line-height: var(--lh-base); }
  .intro { margin: 0; color: var(--text-2); font-size: var(--fs-sm); line-height: var(--lh-base); }
  .strip { padding: var(--sp-2) var(--sp-3) 0; flex-shrink: 0; }
  .search-field.list-search { width: auto; margin: var(--sp-2) var(--sp-3); flex-shrink: 0; }
  /* Inside the list, whose own side gap (--sp-2) would add to the cards' margin: they keep the edge of the search. */
  .offers { margin: 0 calc(-1 * var(--sp-2)); }
  /* The list scrolls its last row out from under the «+» (.m-fab).
     The bar of sections lies under .content and holds the safe area itself. */
  :global(.list-wrap):has(~ .m-fab) { padding-bottom: var(--fab-room); }
</style>

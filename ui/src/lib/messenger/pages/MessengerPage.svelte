<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import '../shared/emoji/font.css';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Skeleton from '$lib/core/ui/Skeleton.svelte';
  import Avatar from '../contacts/Avatar.svelte';
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
  import ContactList from '../contacts/ContactList.svelte';
  import ContactDetails from '../contacts/ContactDetails.svelte';
  import ContactAddForm from '../contacts/ContactAddForm.svelte';
  import ScanQrButton from '../invites/ScanQrButton.svelte';
  import MyQrButton from '../invites/MyQrButton.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import SettingsView from './SettingsView.svelte';
  import { SETTINGS_TABS, SETTINGS_ICONS, settingsUi } from './settingsTab.svelte';
  import ConfirmHost from '../shared/ConfirmHost.svelte';
  import ReportHost from '../shared/ReportHost.svelte';
  import ServerAcceptDialog from '../servers/ServerAcceptDialog.svelte';
  import ServerInfoDialog from '../servers/ServerInfoDialog.svelte';
  import PrivateInviteDialog from '../servers/PrivateInviteDialog.svelte';
  import RequestsList from '../requests/RequestsList.svelte';
  import RequestView from '../requests/RequestView.svelte';
  import { requestStore, requestsUi } from '../requests/requestStore.svelte';
  import { profileLabel, type MessengerChat } from '../api';
  import { onChatOpened } from '../content/actions';
  import { paletteUi } from '../palette.svelte';
  import { chatKind } from '../chats/sections';
  import CallHistory from '../calls/CallHistory.svelte';
  import CallDetails from '../calls/CallDetails.svelte';
  import NewCallDialog from '../calls/NewCallDialog.svelte';
  import Dialer from '../calls/Dialer.svelte';
  import CallIcon from '../calls/CallIcon.svelte';
  import { callLog } from '../calls/callLog.svelte';
  import { chatSections } from '../seams/sections.svelte';

  /**
   * The sections of the column: the chats between two (`chat`), the groups,
   * the journal of calls, the contacts, the settings, or the id of a section
   * a module gives (seams/sections: the channels).
   */
  type View = 'chat' | 'groups' | 'calls' | 'contacts' | 'settings' | (string & {});
  let view = $state<View>('chat');
  /** The section a module gives, when one is shown, and the row of it open. */
  const host = $derived(chatSections.get(view));
  let sectionChat = $state<string | null>(null);
  /** The dialog of a new row of a module's section, open, by section. */
  let creating = $state<Record<string, boolean>>({});
  // The module was switched off under its section: back to the chats.
  $effect(() => {
    if (!host && !['chat', 'groups', 'calls', 'contacts', 'settings'].includes(view)) untrack(() => { view = 'chat'; sectionChat = null; });
  });
  let query = $state('');
  let newChat = $state(false);
  let newGroup = $state(false);
  let newCall = $state(false);
  /** The chat whose calls the pane of «Звонки» shows. */
  let callChat = $state<string | null>(null);
  /** The row of the journal that opened it. */
  let callRow = $state<string | null>(null);
  /** The contact open in the pane of «Контакты» (its key), the search of their list, the dialog «Новый контакт». */
  let contactKey = $state<string | null>(null);
  let contactQuery = $state('');
  let addingContact = $state(false);
  const contact = $derived(contactKey ? (messengerStore.contacts.find((c) => c.pubkey === contactKey) ?? null) : null);

  /** The section a chat is listed in. */
  const sectionOf = (chatId: string): View => {
    const c = chatStore.chats.find((x) => x.id === chatId);
    return (c ? chatKind(c) : chatId.startsWith('group:') ? 'group' : 'dm') === 'group' ? 'groups' : 'chat';
  };
  const listView = $derived(view === 'chat' || view === 'groups');

  // The journal is looked at while its section is shown: its missed calls are no longer new.
  $effect(() => {
    const on = view === 'calls';
    untrack(() => callLog.show(on).catch(() => {}));
  });
  $effect(() => () => { callLog.shown = false; });

  onMount(() => {
    messengerStore.refresh().catch(() => {});
    messengerStore.startListeners().catch(() => {});
  });
  // A link from outside the module asks for a pane (`?view=settings`: the page «Account»).
  onMount(() => {
    const asked = new URLSearchParams(window.location.search).get('view');
    if (asked === 'settings' || asked === 'contacts' || asked === 'groups' || asked === 'calls' || (asked && chatSections.get(asked))) view = asked;
    const open = new URLSearchParams(window.location.search).get('open');
    if (open && chatSections.get(view)) sectionChat = open;
  });
  // A chat opened from elsewhere (a link, a notification) is shown here, in its section.
  // A row of a module's section (a notice of a channel) opens in that section.
  onMount(() => onChatOpened((id) => {
    const host = chatSections.ofChat(id);
    if (host) { view = host.id; sectionChat = id; } else view = sectionOf(id);
  }));

  const s = $derived(messengerStore.status);
  const connected = $derived(s?.runtime?.relays_connected ?? 0);
  const total = $derived(s?.runtime?.relays_total ?? 0);
  const netState = $derived(netStateOf(s?.runtime));

  /** A chat of the list: a search finds either kind, and the list stays where it is. */
  function openChat(c: MessengerChat) {
    if (!listView) view = sectionOf(c.id);
    chatStore.open(c.id).catch(() => {});
  }

  /** «Написать» of the journal: the chat, in its section. */
  function writeTo(chatId: string) {
    view = sectionOf(chatId);
    callChat = null;
    chatStore.open(chatId).catch(() => {});
  }

  // A command of the palette asked for a dialog or a pane.
  $effect(() => {
    const r = paletteUi.request;
    if (!r) return;
    paletteUi.request = null;
    if (r === 'newChat') newChat = true;
    else if (r === 'newGroup') newGroup = true;
    else if (r === 'newCall') newCall = true;
    else show(r);
  });

  // The identity was removed from Settings: after the onboarding the chats come back, not that pane.
  $effect(() => {
    if (!messengerStore.identity && view === 'settings') view = 'chat';
  });

  // The rail shows the requests or the chats, and the pane a request or a
  // chat, never one of each: the requests open in the chat view with no
  // chat, and a chat opened from anywhere closes them.
  $effect(() => {
    if (requestsUi.shown) untrack(() => show('chat'));
  });
  $effect(() => {
    if (chatStore.activeId) untrack(() => requestsUi.close());
  });
  const request = $derived(requestsUi.shown ? requestStore.current : null);

  /** Accepted: the request is a chat now, shown as any other. */
  function accepted(chatId: string) {
    requestsUi.close();
    view = 'chat';
    chatStore.open(chatId).catch(() => {});
  }

  function show(v: View) {
    view = v;
    if (v !== 'calls') callChat = null;
    // The open chat stays only in its own section of the list.
    const open = chatStore.activeId;
    if (open && !((v === 'chat' || v === 'groups') && sectionOf(open) === v)) chatStore.close();
    if (v !== 'chat') requestsUi.close();
    if (!chatSections.get(v)) sectionChat = null;
  }
</script>

{#if !s}
  <div class="page"><div class="skel"><Skeleton height={64} size={44} trailing /></div></div>
{:else if !s.compiled}
  <div class="page"><div class="error-msg">{$t('msg_status_not_compiled')}</div></div>
{:else if s.error}
  <div class="page"><div class="error-msg">{$t('msg_status_error', { error: s.error })}</div></div>
{:else if messengerStore.needsOnboarding}
  <div class="page">
    <div class="page-header">
      <h1>{$t('msg_title')}</h1>
    </div>
    <p class="page-sub">{$t('msg_intro')}</p>
    <Onboarding />
  </div>
{:else if s.runtime}
  <div class="page page--fill messenger">
    <div class="shell">
      <!-- The sections of the messenger (the design kit's column): chats, groups, calls, contacts; settings and the network at its foot. -->
      <nav class="sections" aria-label={$t('msg_title')}>
        {#snippet badge(n: number)}
          {#if n > 0}<span class="sec-badge">{n > 99 ? '99+' : n}</span>{/if}
        {/snippet}
        <button class="sec" class:active={view === 'chat'} aria-current={view === 'chat' ? 'page' : undefined} onclick={() => show('chat')} title={$t('msg_nav_chats')}>
          <Icon name="message-circle" size={22} />
          {@render badge(chatStore.unreadOf('dm'))}
        </button>
        <button class="sec" class:active={view === 'groups'} aria-current={view === 'groups' ? 'page' : undefined} onclick={() => show('groups')} title={$t('msg_nav_groups')}>
          <Icon name="users" size={22} />
          {@render badge(chatStore.unreadOf('group'))}
        </button>
        {#each chatSections.hosts as h (h.id)}
          <button class="sec" class:active={view === h.id} aria-current={view === h.id ? 'page' : undefined} onclick={() => show(h.id)} title={$t(h.title)}>
            <Icon name={h.icon} size={22} />
            {@render badge(h.unread())}
          </button>
        {/each}
        <button class="sec" class:active={view === 'calls'} aria-current={view === 'calls' ? 'page' : undefined} onclick={() => show('calls')} title={$t('msg_nav_calls')}>
          <Icon name="phone" size={22} />
          {#if callLog.missed > 0}<span class="sec-badge missed">{callLog.missed > 99 ? '99+' : callLog.missed}</span>{/if}
        </button>
        <button class="sec" class:active={view === 'contacts'} aria-current={view === 'contacts' ? 'page' : undefined} onclick={() => show('contacts')} title={$t('msg_contacts_title')}>
          <Icon name="book-user" size={22} />
        </button>
        <span class="spacer"></span>
        <span class="sec-net" title={$t(`msg_net_${netState}` as 'msg_net_on', { connected: String(connected), total: String(total) })}>
          <span class="net {netState}"></span>{connected}/{total}
        </span>
        <!-- Me: my profile and the settings of the chat (the design kit's column ends with my avatar). -->
        <button class="sec me" class:active={view === 'settings'} aria-current={view === 'settings' ? 'page' : undefined} onclick={() => show('settings')} title={$t('msg_settings_title_full')} aria-label={$t('msg_settings_title_full')}>
          <Avatar url={messengerStore.ownProfile?.picture ?? null} label={profileLabel(messengerStore.ownProfile) || $t('msg_you')} seed={messengerStore.identity?.pubkey ?? null} size={36} />
        </button>
      </nav>
      {#if view === 'settings'}
      <!-- The settings (design kit): their sections in the list's column, the chosen one in the pane. -->
      <aside class="rail setnav hidden-narrow">
        <h2 class="setnav-title">{$t('msg_settings_title_full')}</h2>
        <nav class="setnav-list">
          {#each SETTINGS_TABS as id (id)}
            <button class="setnav-item" class:active={settingsUi.tab === id} aria-current={settingsUi.tab === id ? 'page' : undefined} onclick={() => (settingsUi.tab = id)}>
              <Icon name={SETTINGS_ICONS[id]} size={20} />
              <span>{$t(`msg_settings_tab_${id}` as 'msg_settings_tab_profile')}</span>
            </button>
          {/each}
        </nav>
      </aside>
      {:else if view === 'contacts'}
      <aside class="rail" class:hidden-narrow={!!contact}>
        <div class="rail-head">
          <span class="rail-title">{$t('msg_contacts_title')}</span>
          <span class="spacer"></span>
          <ScanQrButton icon disabled={!s.runtime?.session_active} />
          <MyQrButton icon disabled={!s.runtime?.session_active} />
          <button class="icon accent" onclick={() => (addingContact = true)} title={$t('msg_contacts_new')}><Icon name="user-plus" size={16} /></button>
        </div>
        <div class="search">
          <Icon name="search" size={14} />
          <input type="text" bind:value={contactQuery} placeholder={$t('msg_contacts_search')} spellcheck="false" />
          {#if contactQuery}<button class="icon sm" onclick={() => (contactQuery = '')}><Icon name="x" size={12} /></button>{/if}
        </div>
        <ContactList query={contactQuery} selected={contactKey} onselect={(c) => (contactKey = c.pubkey)} />
      </aside>
      {:else if view === 'calls'}
      <aside class="rail" class:hidden-narrow={!!callChat}>
        <div class="rail-head">
          <span class="rail-title">{$t('msg_nav_calls')}</span>
          <span class="spacer"></span>
          <button class="icon accent" onclick={() => (newCall = true)} title={$t('msg_calls_new')}><CallIcon name="phone" size={17} /></button>
        </div>
        <CallHistory selected={callChat ? callRow : null} onselect={(r) => { callChat = r.head.chat_id; callRow = r.key; }} onchat={writeTo} onnew={() => (newCall = true)} />
      </aside>
      {:else if host}
      <aside class="rail" class:hidden-narrow={!!sectionChat}>
        <div class="rail-head">
          <span class="rail-title">{$t(host.title)}</span>
          <span class="spacer"></span>
          <button class="icon accent" onclick={() => (creating[host.id] = true)} title={$t(host.newTitle)}><Icon name={host.newIcon} size={16} /></button>
        </div>
        <div class="search">
          <Icon name="search" size={14} />
          <input type="text" bind:value={query} placeholder={$t('msg_chats_search')} spellcheck="false" />
          {#if query}<button class="icon sm" onclick={() => (query = '')}><Icon name="x" size={12} /></button>{/if}
        </div>
        {#if host.above}{@const Above = host.above}<Above />{/if}
        <ChatList {query} kind={host.id} selected={sectionChat} onopen={openChat} onopenrow={(id) => (sectionChat = id)} />
      </aside>
      {:else}
      <aside class="rail" class:hidden-narrow={!listView || !!chatStore.active || !!request}>
        <div class="rail-head">
          <span class="rail-title">{view === 'groups' ? $t('msg_nav_groups') : $t('msg_nav_chats')}</span>
          <span class="spacer"></span>
          {#if view === 'groups'}
            <button class="icon accent" onclick={() => (newGroup = true)} title={$t('msg_groups_new')}><Icon name="users-plus" size={16} /></button>
          {:else}
            <button class="icon accent" onclick={() => (newChat = true)} title={$t('msg_newchat_title')}><Icon name="message-circle-plus" size={16} /></button>
          {/if}
        </div>
        {#if requestsUi.shown}
          <RequestsList onopen={(r) => (requestStore.selected = r.peer)} onback={() => requestsUi.close()} />
        {:else}
          <div class="search">
            <Icon name="search" size={14} />
            <input type="text" bind:value={query} placeholder={$t('msg_chats_search')} spellcheck="false" />
            {#if query}<button class="icon sm" onclick={() => (query = '')}><Icon name="x" size={12} /></button>{/if}
          </div>
          {#if view === 'groups'}<InvitesBar />{:else}<BridgeOffer />{/if}
          <ChatList {query} kind={view === 'groups' ? 'group' : 'dm'} onopen={openChat} />
        {/if}
      </aside>
      {/if}

      <main class="pane" class:hidden-narrow={(listView && !chatStore.active && !request) || (view === 'calls' && !callChat) || (!!host && !sectionChat)}>
        {#if host}
          {#if sectionChat}
            {@const View = host.view}
            <View chatId={sectionChat} onback={() => (sectionChat = null)} onopen={(id) => (sectionChat = id)} />
          {:else}
            <div class="welcome">
              <div class="welcome-icon"><Icon name={host.icon} size={32} /></div>
              <p>{$t(host.welcomeTitle)}</p>
              <span>{$t(host.welcomeText)}</span>
              <div class="welcome-actions">
                <button class="btn btn-primary" onclick={() => (creating[host.id] = true)}><Icon name={host.newIcon} size={14} />{$t(host.newTitle)}</button>
              </div>
            </div>
          {/if}
        {:else if view === 'calls'}
          {#if callChat}
            <div class="pane-scroll">
              <button class="back-narrow btn btn-ghost btn-sm" onclick={() => (callChat = null)}><Icon name="arrow-left" size={12} />{$t('msg_back')}</button>
              <CallDetails chatId={callChat} onchat={writeTo} ongone={() => (callChat = null)} />
            </div>
          {:else}
            <div class="welcome">
              <div class="welcome-icon"><CallIcon name="phone" size={30} /></div>
              <p>{$t('msg_calls_pick')}</p>
              <span>{$t('msg_calls_pick_text')}</span>
              <div class="welcome-actions">
                <button class="btn btn-primary" onclick={() => (newCall = true)}><CallIcon name="phone" size={14} />{$t('msg_calls_new')}</button>
              </div>
            </div>
          {/if}
        {:else if view === 'contacts'}
          {#if contact}
            <div class="pane-scroll">
              <button class="back-narrow btn btn-ghost btn-sm" onclick={() => (contactKey = null)}><Icon name="arrow-left" size={12} />{$t('msg_back')}</button>
              <ContactDetails {contact} onchat={(pubkey) => { show('chat'); chatStore.openPeer(pubkey).catch(() => {}); }}
                onopenchat={(chatId) => { show('chat'); chatStore.open(chatId).catch(() => {}); }} onremoved={() => (contactKey = null)} />
            </div>
          {:else}
            <div class="welcome">
              <div class="welcome-icon"><Icon name="book-user" size={32} /></div>
              <p>{$t('msg_contacts_pick')}</p>
              <span>{$t('msg_contacts_text')}</span>
              <div class="welcome-actions">
                <button class="btn btn-primary" onclick={() => (addingContact = true)}><Icon name="user-plus" size={14} />{$t('msg_contacts_new')}</button>
              </div>
            </div>
          {/if}
        {:else if view === 'settings'}
          <header class="pane-head">
            <button class="back-narrow btn btn-ghost btn-sm" onclick={() => show('chat')}><Icon name="arrow-left" size={12} />{$t('msg_back')}</button>
            <h2>{$t(`msg_settings_tab_${settingsUi.tab}` as 'msg_settings_tab_profile')}</h2>
          </header>
          <div class="pane-scroll">
            <SettingsView />
          </div>
        {:else if request}
          <RequestView {request} onback={() => (requestStore.selected = null)} onaccepted={accepted} ondone={() => (requestStore.selected = null)} />
        {:else if requestsUi.shown}
          <div class="welcome">
            <div class="welcome-icon"><Icon name="user-question" size={34} /></div>
            <span>{$t('msg_req_pick')}</span>
          </div>
        {:else if chatStore.active?.kind === "group"}
          <GroupChat chat={chatStore.active} onback={() => chatStore.close()} />
        {:else if chatStore.active}
          <DmChat chat={chatStore.active} onback={() => chatStore.close()} />
        {:else if view === 'groups'}
          <div class="welcome">
            <div class="welcome-icon"><Icon name="users" size={34} /></div>
            <p>{$t('msg_groups_welcome_title')}</p>
            <span>{$t('msg_groups_welcome_text')}</span>
            <div class="welcome-actions">
              <button class="btn btn-primary" onclick={() => (newGroup = true)}><Icon name="users-plus" size={14} />{$t('msg_groups_new')}</button>
            </div>
          </div>
        {:else}
          <div class="welcome">
            <div class="welcome-icon"><Icon name="message-circle" size={34} /></div>
            <p>{$t('msg_welcome_title')}</p>
            <span>{$t('msg_welcome_text')}</span>
            <div class="welcome-actions">
              <button class="btn btn-primary" onclick={() => (newChat = true)}><Icon name="message-circle-plus" size={14} />{$t('msg_newchat_title')}</button>
            </div>
          </div>
        {/if}
      </main>
    </div>
  </div>
  <NewChatDialog bind:open={newChat} onopened={() => (view = 'chat')} />
  <NewGroupDialog bind:open={newGroup} onopened={() => (view = 'groups')} />
  <NewCallDialog bind:open={newCall} />
  {#each chatSections.hosts as h (h.id)}
    {@const Create = h.create}
    <Create bind:open={creating[h.id]} onopened={(id) => { view = h.id; sectionChat = id; }} />
  {/each}
  <Dialog bind:open={addingContact} title={$t('msg_contacts_new')} width="min(440px, calc(100vw - 24px))">
    <ContactAddForm stacked onadded={() => (addingContact = false)} />
  </Dialog>
  <Dialer />
  <ConfirmHost />
  <ReportHost />
  <ServerAcceptDialog />
  <ServerInfoDialog />
  <PrivateInviteDialog />
{/if}

<style>
  .skel { width: 352px; max-width: 100%; }
  .messenger { max-width: none; }
  /* Two floating panes 12 apart: the list and the conversation. */
  .shell {
    flex: 1; min-height: 0; display: grid; grid-template-columns: 72px 352px minmax(0, 1fr); gap: var(--sp-3);
  }
  .sections {
    display: flex; flex-direction: column; align-items: center; gap: var(--sp-1); min-height: 0; padding: var(--sp-3) 0;
    border-radius: var(--radius-pane); background: var(--pane); border: 1px solid var(--pane-border);
    box-shadow: var(--pane-highlight), var(--pane-shadow); -webkit-backdrop-filter: var(--pane-filter); backdrop-filter: var(--pane-filter);
  }
  .sec {
    position: relative; display: inline-flex; align-items: center; justify-content: center; width: 44px; height: 44px; padding: 0;
    border: none; border-radius: var(--radius-md); background: none; color: var(--text-soft); cursor: pointer;
    transition: background var(--dur-fast), color var(--dur-fast);
  }
  .sec:hover { background: var(--surface-hover); color: var(--text); }
  .sec.active { background: var(--accent-bg); color: var(--accent-text); box-shadow: inset 0 0 0 1px var(--accent-border); }
  .sec-badge {
    position: absolute; top: 2px; right: 0; min-width: 18px; height: 18px; padding: 0 4px; border-radius: var(--radius-badge);
    background: var(--accent); color: #fff; font-size: var(--fs-2xs); font-weight: var(--fw-bold);
    display: inline-flex; align-items: center; justify-content: center; box-shadow: 0 0 0 2px var(--chrome);
  }
  .sec-badge.missed { background: var(--danger); }
  .sec.me { width: 44px; height: 44px; border-radius: 50%; padding: 0; }
  .sec.me.active { box-shadow: 0 0 0 2px var(--accent); background: none; }
  .sec-net { display: inline-flex; align-items: center; gap: 4px; margin-bottom: var(--sp-1); font-size: var(--fs-2xs); font-weight: var(--fw-semibold); color: var(--text-2); }
  .sec-net .net { margin-left: 0; }
  .rail { display: flex; flex-direction: column; min-height: 0; border-radius: var(--radius-pane); overflow: hidden; background: var(--pane); border: 1px solid var(--pane-border); box-shadow: var(--pane-highlight), var(--pane-shadow); -webkit-backdrop-filter: var(--pane-filter); backdrop-filter: var(--pane-filter); }
  .rail-head { display: flex; align-items: center; gap: 2px; padding: var(--sp-3) var(--sp-3) var(--sp-2) var(--sp-4); min-height: 64px; }
  /* One line in every language: the badge stays beside the name. */
  .rail-title { display: inline-flex; align-items: center; white-space: nowrap; flex-shrink: 0; font-weight: var(--fw-bold); font-size: var(--fs-xl); }
  .spacer { flex: 1; }
  .net { width: 8px; height: 8px; border-radius: 50%; margin-left: 8px; background: var(--text-3); flex-shrink: 0; }
  .net.on { background: var(--success); box-shadow: 0 0 0 3px var(--success-bg); }
  .net.connecting { background: var(--warn-text); animation: pulse 1.4s ease-in-out infinite; }
  .net.lost { background: var(--danger); }
  .net.silent, .net.locked { background: var(--text-3); }
  @keyframes pulse { 50% { opacity: 0.35; } }
  .icon { border: none; background: none; color: var(--text-soft); cursor: pointer; display: inline-flex; align-items: center; justify-content: center; width: 36px; height: 36px; padding: 0; border-radius: var(--radius); }
  .icon :global(.icon) { width: 18px; height: 18px; }
  .icon:hover { color: var(--text); background: var(--surface-hover); }
  .icon.active { color: var(--accent-text-2); background: var(--accent-tint); }
  .icon.accent { color: var(--accent-text-2); }
  .icon.sm { width: 24px; height: 24px; }
  /* The QR buttons of the contacts (their own components) look like the head's icons. */
  .rail-head :global(.ibtn) { border: none; background: none; color: var(--text-soft); cursor: pointer; display: inline-flex; align-items: center; justify-content: center; width: 36px; height: 36px; padding: 0; border-radius: var(--radius); }
  .rail-head :global(.ibtn:hover:not(:disabled)) { color: var(--text); background: var(--surface-hover); }
  .rail-head :global(.ibtn:disabled) { opacity: 0.45; cursor: default; }
  .search {
    display: flex; align-items: center; gap: 8px; min-height: 40px; margin: 0 var(--sp-3) var(--sp-2); padding: 0 var(--sp-4);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius-pill); color: var(--text-3);
  }
  .search:focus-within { border-color: var(--accent-border); }
  /* The pill is the field: its border is the one focus mark. */
  .search input { flex: 1; min-width: 0; border: none; border-radius: 0; background: none; outline: none; box-shadow: none; font: inherit; font-size: var(--fs-base); color: var(--text); padding: 7px 0; }
  .pane { min-width: 0; min-height: 0; display: flex; flex-direction: column; border-radius: var(--radius-pane); overflow: hidden; background: var(--pane); border: 1px solid var(--pane-border); box-shadow: var(--pane-highlight), var(--pane-shadow); -webkit-backdrop-filter: var(--pane-filter); backdrop-filter: var(--pane-filter); }
  .setnav { padding: var(--sp-5) var(--sp-2) var(--sp-3); gap: var(--sp-4); overflow-y: auto; }
  .setnav-title { padding: 0 var(--sp-3); font-size: var(--fs-xl); font-weight: var(--fw-bold); line-height: 1.4; }
  .setnav-list { display: flex; flex-direction: column; gap: 2px; }
  .setnav-item {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 40px; padding: 0 var(--sp-3);
    border: none; border-radius: var(--radius); background: none; color: var(--text-2);
    font: inherit; font-size: var(--fs-base); font-weight: var(--fw-semibold); text-align: left; cursor: pointer;
    transition: background var(--dur-fast), color var(--dur-fast);
  }
  .setnav-item:hover { background: var(--surface-hover); color: var(--text); }
  .setnav-item.active { background: var(--accent-bg); color: var(--accent-text); }
  .pane-head {
    display: flex; align-items: center; gap: var(--sp-3); flex-shrink: 0; min-height: 64px; padding: 0 var(--sp-6);
    background: var(--pane-head); border-bottom: 1px solid var(--pane-border);
  }
  .pane-head h2 { margin: 0; font-size: var(--fs-xl); font-weight: var(--fw-bold); }
  .pane-scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-5); display: flex; flex-direction: column; gap: var(--sp-4); }
  .back-narrow { display: none; align-self: flex-start; }
  .welcome { margin: auto; display: flex; flex-direction: column; align-items: center; gap: var(--sp-3); text-align: center; padding: var(--sp-6); color: var(--text-3); }
  .welcome-icon { width: 72px; height: 72px; border-radius: 50%; background: var(--accent-tint); color: var(--accent-text-2); display: flex; align-items: center; justify-content: center; }
  .welcome p { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); }
  .welcome span { font-size: var(--fs-sm); max-width: 360px; line-height: 1.5; color: var(--text-2); }
  .welcome-actions { display: flex; gap: var(--sp-2); flex-wrap: wrap; justify-content: center; }

  @media (max-width: 860px) {
    .shell { grid-template-columns: 72px minmax(0, 1fr); }
    .hidden-narrow { display: none; }
    .back-narrow { display: inline-flex; }
  }
</style>

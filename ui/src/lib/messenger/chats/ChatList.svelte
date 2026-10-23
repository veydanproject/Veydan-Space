<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { drafts } from '../shared/drafts.svelte';
  import { toast } from '$lib/core/ui/toast.svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { chatStore } from './chatStore.svelte';
  import { presenceStore } from '../presence/presenceStore.svelte';
  import { listStamp } from '../shared/time';
  import { confirmStore } from '../shared/confirm.svelte';
  import { longpress } from '../shared/longpress';
  import { chatMatches, contactHaystacks } from '../shared/search';
  import { messengerStore } from '../store.svelte';
  import { botCardLineKey } from '../content/botCard';
  import { bodyLine } from '../push/wording';
  import { botCards } from '../seams/cards.svelte';
  import type { MessengerChat } from '../api';
  import { goto } from '$app/navigation';
  import { requestStore, requestsUi } from '../requests/requestStore.svelte';
  import { requestsHref } from '../mobile/routes';
  import { onPhone } from '../shared/phone';
  import { inSection, type ChatSection } from './sections';
  import { groupStore } from '../groups/groupStore.svelte';
  import { answersRequests } from '../groups/permissions';
  import ServerLabel from '../servers/ServerLabel.svelte';
  import { chatSections, type SectionRow } from '../seams/sections.svelte';

  interface Props {
    query: string;
    /** The section listed while nothing is typed: the chats between two, the groups, or a section a module gives (seams/sections). */
    kind?: ChatSection | string;
    onopen: (chat: MessengerChat) => void;
    /** A row of a module's section opened, by its chat id. */
    onopenrow?: (chatId: string) => void;
    /** The row of a module's section open now. */
    selected?: string | null;
  }
  let { query, kind = 'dm', onopen, onopenrow, selected = null }: Props = $props();

  /** The section a module gives, when `kind` names one: its rows instead of the messenger's chats. */
  const host = $derived(kind === 'dm' || kind === 'group' ? null : chatSections.get(kind));

  const phone = onPhone();

  /** Requests to join a group of mine that wait for my answer: a manager's count, apart from the unread. */
  function asking(c: MessengerChat): number {
    const g = c.kind === 'group' ? groupStore.byChat(c.id) : null;
    return answersRequests(g) ? g.requests.length : 0;
  }

  let showArchived = $state(false);
  let menu = $state<{ open: boolean; x: number; y: number; chat: MessengerChat | null }>({ open: false, x: 0, y: 0, chat: null });

  const q = $derived(query.trim());
  const haystacks = $derived(contactHaystacks(messengerStore.contacts));
  const matches = (c: MessengerChat) => !host && inSection(c, kind as ChatSection, q) && chatMatches(c, q, haystacks);
  const rowMatches = (r: SectionRow) => !q || r.title.toLocaleLowerCase().includes(q.toLocaleLowerCase());
  const byTime = (a: SectionRow, b: SectionRow) => Number(b.pinned) - Number(a.pinned) || (b.at ?? 0) - (a.at ?? 0);
  const hostRows = $derived(host ? host.rows().filter(rowMatches).sort(byTime) : []);
  const hostVisible = $derived(hostRows.filter((r) => !r.archived));
  const hostArchived = $derived(hostRows.filter((r) => r.archived));
  let rowMenu = $state<{ open: boolean; x: number; y: number; row: SectionRow | null }>({ open: false, x: 0, y: 0, row: null });

  /** A choice the messenger keeps of a module's row, then the module reads its rows again. */
  function rowAct(run: () => Promise<unknown>) {
    act(async () => { await run(); host?.refresh(); });
  }

  const rowItems = $derived.by((): MenuEntry[] => {
    const r = rowMenu.row;
    if (!r) return [];
    return [
      { label: r.pinned ? $t('msg_chat_unpin') : $t('msg_chat_pin'), icon: 'pin', onselect: () => rowAct(() => chatStore.setPinned(r.chatId, !r.pinned)) },
      { label: r.muted ? $t('msg_chat_unmute') : $t('msg_chat_mute'), icon: r.muted ? 'bell' : 'bell-off', onselect: () => rowAct(() => chatStore.setMuted(r.chatId, !r.muted)) },
      { label: r.archived ? $t('msg_chat_unarchive') : $t('msg_chat_archive'), icon: r.archived ? 'archive-restore' : 'archive', onselect: () => rowAct(() => chatStore.setArchived(r.chatId, !r.archived)) },
    ];
  });
  // A request of a stranger is not a chat yet: it waits behind the row of
  // requests, out of the list and the archive, until it is accepted.
  const isRequest = (c: MessengerChat) => c.mode === 'request_received';
  const visible = $derived(chatStore.chats.filter((c) => !c.archived && !isRequest(c) && matches(c)));
  const archived = $derived(chatStore.chats.filter((c) => c.archived && !isRequest(c) && matches(c)));
  // The requests of strangers are chats between two: their row is of «Чаты».
  const requests = $derived(kind === 'dm' ? requestStore.count : 0);

  /** The desktop shows the requests in the rail in place of the chats; a phone goes to their screen. */
  function openRequests() {
    if (phone) void goto(requestsHref());
    else requestsUi.open();
  }

  /** The line under the name: the newest message, or a card of the bot of Veydan Cloud worded by its type. */
  function preview(c: MessengerChat): string {
    // A group the network closed has nothing new to show; the row says so.
    if (c.kind === 'group' && c.banned) return $t('msg_group_removed_short');
    // A photo is a photo, a voice message a voice message (as a push says), not the name of a file.
    if (c.last_body) return bodyLine(c.last_body, $t);
    if (c.last_preview != null) return c.last_preview;
    const card = chatStore.tailCard(c);
    return card ? $t(botCardLineKey(card, botCards.host)) : $t('msg_chat_no_messages');
  }

  function openMenu(e: MouseEvent, chat: MessengerChat) {
    e.preventDefault();
    menu = { open: true, x: e.clientX, y: e.clientY, chat };
  }

  /** An action of the menu: a failure says so, with «Повторить». */
  function act(run: () => Promise<unknown>) {
    run().catch(() => toast.error($t('msg_action_failed'), { onAction: () => act(run) }));
  }
  /** Into the archive, with «Отменить» for 8 s. */
  function archive(c: MessengerChat) {
    act(async () => {
      await chatStore.setArchived(c.id, true);
      toast.undo($t('msg_chat_archived_toast'), { icon: 'archive', onAction: () => act(() => chatStore.setArchived(c.id, false)) });
    });
  }

  const items = $derived.by((): MenuEntry[] => {
    const c = menu.chat;
    if (!c) return [];
    const group = c.kind === "group";
    const removal: MenuEntry[] = group
      // A group I am in is left from its own panel, not thrown away here.
      ? (c.can_send ? [] : [{ type: "separator" }, { label: $t("msg_group_forget"), icon: "trash-2", danger: true, onselect: async () => { if (await confirmStore.ask($t("msg_group_forget_confirm", { name: c.title }), $t("msg_group_forget"), true)) chatStore.deleteChat(c.id).catch(() => {}); } }])
      : [{ type: "separator" }, { label: $t("msg_chat_delete"), icon: "trash-2", danger: true, onselect: async () => { if (await confirmStore.ask($t("msg_chat_delete_confirm", { name: c.title }), $t("msg_chat_delete"), true)) chatStore.deleteChat(c.id); } }];
    return [
      { label: c.pinned ? $t('msg_chat_unpin') : $t('msg_chat_pin'), icon: 'pin', onselect: () => act(() => chatStore.setPinned(c.id, !c.pinned)) },
      { label: c.is_muted ? $t('msg_chat_unmute') : $t('msg_chat_mute'), icon: c.is_muted ? 'bell' : 'bell-off', onselect: () => act(() => chatStore.setMuted(c.id, !c.is_muted)) },
      { label: $t('msg_chat_mark_read'), icon: 'check-check', disabled: c.unread === 0, onselect: () => act(() => chatStore.markRead(c.id)) },
      { label: c.archived ? $t('msg_chat_unarchive') : $t('msg_chat_archive'), icon: c.archived ? 'archive-restore' : 'archive', onselect: () => (c.archived ? act(() => chatStore.setArchived(c.id, false)) : archive(c)) },
      ...removal,
    ];
  });
</script>

{#snippet extRow(r: SectionRow)}
  <li>
    <button class="chat" class:active={selected === r.chatId} onclick={() => onopenrow?.(r.chatId)} oncontextmenu={(e) => { e.preventDefault(); rowMenu = { open: true, x: e.clientX, y: e.clientY, row: r }; }}
      use:longpress={{ onpress: (p) => (rowMenu = { open: true, x: p.x, y: p.y, row: r }) }}>
      <!-- A picture the module already has (a `data:` URL) is shown as it is; an address is fetched by the runtime. -->
      <Avatar url={r.picture?.startsWith('data:') ? null : r.picture} src={r.picture?.startsWith('data:') ? r.picture : null} label={r.title} seed={r.seed} size={42} />
      <span class="body">
        <span class="top">
          {#if host}<span class="dim"><Icon name={host.icon} size={12} /></span>{/if}
          <span class="title">{r.title}</span>
          {#if r.muted}<span class="dim"><Icon name="bell-off" size={12} /></span>{/if}
          {#if r.pinned}<span class="dim"><Icon name="pin" size={12} /></span>{/if}
          <span class="stamp">{listStamp(r.at)}</span>
        </span>
        <span class="bottom">
          <span class="preview">{r.preview}</span>
          {#if r.asks}<span class="badge asks"><Icon name="user-plus" size={11} />{r.asks > 99 ? '99+' : r.asks}</span>{/if}
          {#if r.unread > 0}<span class="badge" class:muted={r.muted}>{r.unread > 99 ? '99+' : r.unread}</span>{/if}
        </span>
      </span>
    </button>
  </li>
{/snippet}

{#snippet row(c: MessengerChat)}
  <li>
    <button class="chat" class:active={chatStore.activeId === c.id} onclick={() => onopen(c)} oncontextmenu={(e) => openMenu(e, c)}
      use:longpress={{ onpress: (p) => (menu = { open: true, x: p.x, y: p.y, chat: c }) }}>
      <Avatar url={c.picture} label={c.title} seed={c.peer_pubkey ?? c.id} size={42} online={c.kind === 'dm' && presenceStore.status(c.peer_pubkey) === 'online'} />
      <span class="body">
        <span class="top">
          {#if c.kind === "group"}<span class="dim"><Icon name="users" size={12} /></span>{/if}
          <span class="title">{c.title}</span>
          {#if c.server_id}<ServerLabel server={c.server_id} />{/if}
          {#if c.is_muted}<span class="dim"><Icon name="bell-off" size={12} /></span>{/if}
          {#if c.pinned}<span class="dim"><Icon name="pin" size={12} /></span>{/if}
          <span class="stamp">{listStamp(c.last_message_at)}</span>
        </span>
        <span class="bottom">
          {#if drafts.get(c.id)?.trim() && chatStore.activeId !== c.id}
            <span class="preview"><span class="draft">{$t('msg_chat_draft')}</span> {drafts.get(c.id)}</span>
          {:else}
            <span class="preview">{preview(c)}</span>
          {/if}
          {#if asking(c)}
            <span class="badge asks" title={$t('msg_greq_badge', { n: String(asking(c)) })} aria-label={$t('msg_greq_badge', { n: String(asking(c)) })}>
              <Icon name="user-plus" size={11} />{asking(c) > 99 ? '99+' : asking(c)}
            </span>
          {/if}
          {#if c.unread > 0}<span class="badge" class:muted={c.is_muted}>{c.unread > 99 ? '99+' : c.unread}</span>{/if}
        </span>
      </span>
    </button>
  </li>
{/snippet}

<div class="list-wrap">
  {#if requests > 0}
    <!-- No unread badge: a request must not call anyone back to the app. -->
    <button class="chat requests" onclick={openRequests}>
      <span class="req-icon"><Icon name="user-question" size={20} /></span>
      <span class="body">
        <span class="top"><span class="title">{$t('msg_req_row', { n: String(requests) })}</span></span>
        <span class="bottom"><span class="preview">{$t('msg_req_row_hint')}</span></span>
      </span>
      <span class="dim"><Icon name="chevron-right" size={16} /></span>
    </button>
  {/if}
  {#if host}
    {#if hostVisible.length === 0 && hostArchived.length === 0 && !q && host.loading?.()}
      <div class="empty loading" role="status">
        <span class="spinner" aria-hidden="true"></span>
        {#if host.loadingTitle}<p>{$t(host.loadingTitle)}</p>{/if}
      </div>
    {:else if hostVisible.length === 0 && hostArchived.length === 0}
      <div class="empty">
        <Icon name={host.icon} size={28} />
        <p>{q ? $t('msg_chats_nothing_found') : $t(host.emptyTitle)}</p>
        {#if !q}<span>{$t(host.emptyHint)}</span>{/if}
      </div>
    {:else}
      <ul class="list divided">
        {#each hostVisible as r (r.chatId)}{@render extRow(r)}{/each}
      </ul>
      {#if hostArchived.length}
        <button class="archived-toggle" onclick={() => (showArchived = !showArchived)}>
          <Icon name="archive" size={13} />
          {$t('msg_chats_archived', { n: String(hostArchived.length) })}
          <Icon name={showArchived ? 'chevron-up' : 'chevron-down'} size={13} />
        </button>
        {#if showArchived}
          <ul class="list divided">
            {#each hostArchived as r (r.chatId)}{@render extRow(r)}{/each}
          </ul>
        {/if}
      {/if}
    {/if}
  {:else if visible.length === 0 && archived.length === 0}
    <div class="empty">
      <Icon name={kind === 'group' ? 'users' : 'message-circle'} size={28} />
      <p>{q ? $t('msg_chats_nothing_found') : kind === 'group' ? $t('msg_groups_empty') : $t('msg_chats_empty')}</p>
      {#if !q}<span>{kind === 'group' ? $t('msg_groups_empty_hint') : $t('msg_chats_empty_hint')}</span>{/if}
    </div>
  {:else}
    <ul class="list divided">
      {#each visible as c (c.id)}{@render row(c)}{/each}
    </ul>
    {#if archived.length}
      <button class="archived-toggle" onclick={() => (showArchived = !showArchived)}>
        <Icon name="archive" size={13} />
        {$t('msg_chats_archived', { n: String(archived.length) })}
        <Icon name={showArchived ? 'chevron-up' : 'chevron-down'} size={13} />
      </button>
      {#if showArchived}
        <ul class="list divided">
          {#each archived as c (c.id)}{@render row(c)}{/each}
        </ul>
      {/if}
    {/if}
  {/if}
</div>

<ContextMenu bind:open={menu.open} x={menu.x} y={menu.y} {items} onclose={() => (menu.open = false)} />
<ContextMenu bind:open={rowMenu.open} x={rowMenu.x} y={rowMenu.y} items={rowItems} onclose={() => (rowMenu.open = false)} />

<style>
  .list-wrap { flex: 1; min-height: 0; overflow-y: auto; padding: 0 var(--sp-2) var(--sp-2); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; --rows-gap: 2px; }
  .chat {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; text-align: left;
    min-height: 64px; padding: var(--sp-2) var(--sp-3); border: none; border-radius: var(--radius-lg);
    background: none; color: inherit; font: inherit; cursor: pointer;
    transition: background var(--dur-fast) var(--ease);
  }
  .chat:hover { background: var(--surface-hover); }
  .chat.active { background: var(--accent-bg); box-shadow: inset 0 0 0 1px var(--accent-border); }
  .requests { margin-bottom: 2px; }
  .req-icon {
    width: 42px; height: 42px; border-radius: 50%; flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center;
    background: var(--surface-3); color: var(--text-2);
  }
  .body { display: flex; flex-direction: column; gap: 3px; min-width: 0; flex: 1; }
  .top, .bottom { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .title { font-weight: var(--fw-semibold); font-size: var(--fs-body); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dim { color: var(--text-3); display: inline-flex; flex-shrink: 0; }
  .stamp { margin-left: auto; font-size: var(--fs-xs); color: var(--text-2); flex-shrink: 0; }
  .draft { color: var(--danger-text); }
  .preview { font-size: var(--fs-sm); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
  .badge {
    flex-shrink: 0; min-width: 20px; height: 20px; padding: 0 var(--sp-1); border-radius: var(--radius-badge);
    background: var(--accent); color: #fff; font-size: var(--fs-2xs); font-weight: var(--fw-bold);
    display: inline-flex; align-items: center; justify-content: center;
  }
  .badge.muted { background: var(--surface-3); color: var(--text-2); }
  /* Requests to join wait: the accent's tint, so the unread count stays the one that calls. */
  .badge.asks { gap: 3px; background: var(--accent-tint); color: var(--accent-text); box-shadow: inset 0 0 0 1px var(--accent-tint-border); }
  .empty {
    display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); text-align: center;
    padding: var(--sp-8) var(--sp-4); color: var(--text-3);
  }
  .empty p { margin: 0; color: var(--text-body); font-weight: var(--fw-semibold); font-size: var(--fs-sm); }
  .empty span { font-size: var(--fs-xs); line-height: 1.5; }
  .empty.loading .spinner { width: 20px; height: 20px; }
  .archived-toggle {
    display: flex; align-items: center; gap: 6px; width: 100%; margin-top: var(--sp-2);
    padding: var(--sp-2) var(--sp-3); border: none; background: none; cursor: pointer;
    color: var(--text-3); font: inherit; font-size: var(--fs-xs);
  }
  .archived-toggle:hover { color: var(--text-2); }
</style>

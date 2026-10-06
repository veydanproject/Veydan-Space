<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { chatStore } from './chatStore.svelte';
  import { presenceStore } from '../presence/presenceStore.svelte';
  import { listStamp } from '../shared/time';
  import { confirmStore } from '../shared/confirm.svelte';
  import { longpress } from '../shared/longpress';
  import type { MessengerChat } from '../api';

  interface Props {
    query: string;
    onopen: (chat: MessengerChat) => void;
  }
  let { query, onopen }: Props = $props();

  let showArchived = $state(false);
  let menu = $state<{ open: boolean; x: number; y: number; chat: MessengerChat | null }>({ open: false, x: 0, y: 0, chat: null });

  const q = $derived(query.trim().toLowerCase());
  const matches = (c: MessengerChat) =>
    !q || c.title.toLowerCase().includes(q) || (c.peer_npub ?? '').includes(q) || (c.last_preview ?? '').toLowerCase().includes(q);
  const visible = $derived(chatStore.chats.filter((c) => !c.archived && matches(c)));
  const archived = $derived(chatStore.chats.filter((c) => c.archived && matches(c)));

  function openMenu(e: MouseEvent, chat: MessengerChat) {
    e.preventDefault();
    menu = { open: true, x: e.clientX, y: e.clientY, chat };
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
      { label: c.pinned ? $t('msg_chat_unpin') : $t('msg_chat_pin'), icon: 'pin', onselect: () => chatStore.setPinned(c.id, !c.pinned) },
      { label: c.is_muted ? $t('msg_chat_unmute') : $t('msg_chat_mute'), icon: c.is_muted ? 'bell' : 'bell-off', onselect: () => chatStore.setMuted(c.id, !c.is_muted) },
      { label: $t('msg_chat_mark_read'), icon: 'check-check', disabled: c.unread === 0, onselect: () => chatStore.markRead(c.id) },
      { label: c.archived ? $t('msg_chat_unarchive') : $t('msg_chat_archive'), icon: c.archived ? 'archive-restore' : 'archive', onselect: () => chatStore.setArchived(c.id, !c.archived) },
      ...removal,
    ];
  });
</script>

{#snippet row(c: MessengerChat)}
  <li>
    <button class="chat" class:active={chatStore.activeId === c.id} onclick={() => onopen(c)} oncontextmenu={(e) => openMenu(e, c)}
      use:longpress={{ onpress: (p) => (menu = { open: true, x: p.x, y: p.y, chat: c }) }}>
      <Avatar url={c.picture} label={c.title} seed={c.peer_pubkey ?? c.id} size={42} online={c.kind === 'dm' && presenceStore.status(c.peer_pubkey) === 'online'} />
      <span class="body">
        <span class="top">
          {#if c.kind === "group"}<span class="dim"><Icon name="users" size={12} /></span>{/if}
          <span class="title">{c.title}</span>
          {#if c.is_muted}<span class="dim"><Icon name="bell-off" size={12} /></span>{/if}
          {#if c.pinned}<span class="dim"><Icon name="pin" size={12} /></span>{/if}
          <span class="stamp">{listStamp(c.last_message_at)}</span>
        </span>
        <span class="bottom">
          <span class="preview">{c.last_preview ?? $t('msg_chat_no_messages')}</span>
          {#if c.unread > 0}<span class="badge" class:muted={c.is_muted}>{c.unread > 99 ? '99+' : c.unread}</span>{/if}
        </span>
      </span>
    </button>
  </li>
{/snippet}

<div class="list-wrap">
  {#if visible.length === 0 && archived.length === 0}
    <div class="empty">
      <Icon name="message-circle" size={28} />
      <p>{q ? $t('msg_chats_nothing_found') : $t('msg_chats_empty')}</p>
      {#if !q}<span>{$t('msg_chats_empty_hint')}</span>{/if}
    </div>
  {:else}
    <ul class="list">
      {#each visible as c (c.id)}{@render row(c)}{/each}
    </ul>
    {#if archived.length}
      <button class="archived-toggle" onclick={() => (showArchived = !showArchived)}>
        <Icon name="archive" size={13} />
        {$t('msg_chats_archived', { n: String(archived.length) })}
        <Icon name={showArchived ? 'chevron-up' : 'chevron-down'} size={13} />
      </button>
      {#if showArchived}
        <ul class="list">
          {#each archived as c (c.id)}{@render row(c)}{/each}
        </ul>
      {/if}
    {/if}
  {/if}
</div>

<ContextMenu bind:open={menu.open} x={menu.x} y={menu.y} {items} onclose={() => (menu.open = false)} />

<style>
  .list-wrap { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-1); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .chat {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; text-align: left;
    padding: var(--sp-2) var(--sp-3); border: none; border-radius: var(--radius-md);
    background: none; color: inherit; font: inherit; cursor: pointer;
    transition: background var(--dur-fast) var(--ease);
  }
  .chat:hover { background: var(--surface-row-hover); }
  .chat.active { background: var(--accent-tint); }
  .body { display: flex; flex-direction: column; gap: 3px; min-width: 0; flex: 1; }
  .top, .bottom { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .title { font-weight: var(--fw-semibold); font-size: var(--fs-sm); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dim { color: var(--text-3); display: inline-flex; flex-shrink: 0; }
  .stamp { margin-left: auto; font-size: var(--fs-2xs); color: var(--text-3); flex-shrink: 0; }
  .preview { font-size: var(--fs-xs); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
  .badge {
    flex-shrink: 0; min-width: 18px; height: 18px; padding: 0 5px; border-radius: var(--radius-pill);
    background: var(--accent); color: #fff; font-size: var(--fs-2xs); font-weight: var(--fw-bold);
    display: inline-flex; align-items: center; justify-content: center;
  }
  .badge.muted { background: var(--surface-3); color: var(--text-2); }
  .empty {
    display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); text-align: center;
    padding: var(--sp-8) var(--sp-4); color: var(--text-3);
  }
  .empty p { margin: 0; color: var(--text-body); font-weight: var(--fw-semibold); font-size: var(--fs-sm); }
  .empty span { font-size: var(--fs-xs); line-height: 1.5; }
  .archived-toggle {
    display: flex; align-items: center; gap: 6px; width: 100%; margin-top: var(--sp-2);
    padding: var(--sp-2) var(--sp-3); border: none; background: none; cursor: pointer;
    color: var(--text-3); font: inherit; font-size: var(--fs-xs);
  }
  .archived-toggle:hover { color: var(--text-2); }
</style>

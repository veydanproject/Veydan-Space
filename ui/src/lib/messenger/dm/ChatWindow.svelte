<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { tick, type Snippet } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import MessageBubble from './MessageBubble.svelte';
  import AlbumBubble from './AlbumBubble.svelte';
  import Timeline from '../content/Timeline.svelte';
  import type { TimelineItem } from '../content/types';
  import { tint } from '../shared/tint';
  import Composer from './Composer.svelte';
  import RelationBanner from './RelationBanner.svelte';
  import MediaBubble from '../media/MediaBubble.svelte';
  import AttachButton from '../media/AttachButton.svelte';
  import MediaViewer from '../media/MediaViewer.svelte';
  import type { MessengerRecording } from '../api';
  import { chatStore } from '../chats/chatStore.svelte';
  import { messengerStore } from '../store.svelte';
  import { confirmStore } from '../shared/confirm.svelte';
  import { onKeyboard } from '../shared/keyboard';
  import { onPhone } from '../shared/phone';
  import { dmErrorCode, mediaErrorCode, messengerError, type DmAction, type MessengerChat, type MessengerMessage } from '../api';

  interface Props {
    chat: MessengerChat;
    onback?: () => void;
    actions?: Snippet;
    banner?: Snippet;
    /** Replaces the composer when the chat cannot be written to. */
    footer?: Snippet;
    /** Replaces the line under the title (a group: how many members). */
    subtitle?: Snippet;
    /** Chats of many: who wrote a message. */
    author?: (pubkey: string) => string;
    /** Text of a system line, when it is not about the relationship. */
    systemText?: (m: MessengerMessage) => string | null;
    /** Entries of the chat menu that replace the relationship ones. */
    chatEntries?: MenuEntry[];
    /** Entries of the chat menu put before all others. */
    leadEntries?: MenuEntry[];
    /** May this message of someone else be removed for everyone? */
    canModerate?: (m: MessengerMessage) => boolean;
    /** Explains refusals this window does not know. */
    explainError?: (e: unknown) => string | null;
    ontitle?: () => void;
  }
  let { chat, onback, actions, banner, footer, subtitle, author, systemText, chatEntries, leadEntries, canModerate, explainError, ontitle }: Props = $props();
  const isGroup = $derived(chat.kind === "group");
  const canAttach = $derived(chat.mode === "full_chat" || chat.mode === "group");
  /** A phone screen: its back is a chevron, its header the bar colour, no keyboard on its own. */
  const phone = onPhone();

  let scroller = $state<HTMLDivElement | null>(null);
  let content = $state<HTMLDivElement | null>(null);
  let replyTo = $state<MessengerMessage | null>(null);
  let editing = $state<MessengerMessage | null>(null);
  let error = $state('');
  let atBottom = $state(true);
  let highlighted = $state<string | null>(null);
  let menu = $state<{ open: boolean; x: number; y: number; m: MessengerMessage | null }>({ open: false, x: 0, y: 0, m: null });
  let chatMenu = $state<{ open: boolean; x: number; y: number }>({ open: false, x: 0, y: 0 });
  let acting = $state(false);

  const online = $derived((messengerStore.status?.runtime?.relays_connected ?? 0) > 0);
  const sessionActive = $derived(!!messengerStore.status?.runtime?.session_active);

  // Leaving a chat drops reply/edit state.
  let lastChat = '';
  $effect(() => {
    if (chat.id !== lastChat) { lastChat = chat.id; replyTo = null; editing = null; error = ''; atBottom = true; }
  });

  // New message at the tail: follow it when the user is already at the bottom.
  $effect(() => {
    void chatStore.tailTick;
    if (atBottom) tick().then(scrollToBottom);
  });

  // What is shown grows after it is drawn (a card, a picture): the latest message stays in view.
  $effect(() => {
    if (!content) return;
    const watch = new ResizeObserver(() => { if (atBottom) scrollToBottom(); });
    watch.observe(content);
    return () => watch.disconnect();
  });

  // The keyboard takes height away: keep the latest message in view.
  $effect(() => onKeyboard(() => { if (atBottom) tick().then(scrollToBottom); }));

  /** Height of what is shown when it was last looked at. */
  let seenHeight = 0;

  function scrollToBottom() {
    if (!scroller) return;
    seenHeight = scroller.scrollHeight;
    scroller.scrollTop = scroller.scrollHeight;
  }

  async function onscroll() {
    if (!scroller) return;
    // The distance grew because the content did, not because the user left.
    if (atBottom && scroller.scrollHeight !== seenHeight) { scrollToBottom(); return; }
    seenHeight = scroller.scrollHeight;
    atBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
    if (scroller.scrollTop < 120 && chatStore.hasMore && !chatStore.loadingOlder) {
      const before = scroller.scrollHeight;
      await chatStore.loadOlder();
      await tick();
      // Keep the viewport on the same message after older ones are prepended.
      if (scroller) scroller.scrollTop += scroller.scrollHeight - before;
    }
  }

  /** Relationship refusals arrive as stable codes; everything else as text. */
  function explain(e: unknown): string {
    const own = explainError?.(e);
    if (own) return own;
    const code = dmErrorCode(e);
    if (code) return $t(`msg_err_${code}` as "msg_err_dm_blocked", { name: chat.title });
    const media = mediaErrorCode(e);
    return media ? $t(`msg_media_${media.replace(".", "_")}` as "msg_media_err_network") : messengerError(e);
  }

  async function act(a: DmAction) {
    if (a === "block" && !(await confirmStore.ask($t("msg_rel_confirm_block", { name: chat.title }), $t("msg_rel_cta_block"), true))) return;
    if (a === "remove" && chat.is_contact && chat.mode === "full_chat" && !(await confirmStore.ask($t("msg_rel_confirm_remove", { name: chat.title }), $t("msg_rel_cta_remove"), true))) return;
    error = ""; acting = true;
    try { await chatStore.act(chat.id, a); }
    catch (e) { error = explain(e); }
    finally { acting = false; }
  }

  function openChatMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    chatMenu = { open: true, x: r.right - 200, y: r.bottom + 4 };
  }

  const chatItems = $derived.by((): MenuEntry[] => {
    const list: MenuEntry[] = [...(leadEntries ?? [])];
    if (chatEntries) list.push(...chatEntries);
    else if (chat.mode === "blocked") list.push({ label: $t("msg_rel_cta_unblock"), icon: "lock-open", onselect: () => act("unblock") });
    else {
      if (chat.is_contact) list.push({ label: $t("msg_rel_cta_remove"), icon: "user", onselect: () => act("remove") });
      else if (chat.mode !== "request_received") list.push({ label: $t("msg_rel_cta_add"), icon: "user-plus", onselect: () => act("request") });
      list.push({ label: $t("msg_rel_cta_block"), icon: "ban", danger: true, onselect: () => act("block") });
    }
    list.push({ type: "separator" });
    list.push({ label: chat.pinned ? $t("msg_chat_unpin") : $t("msg_chat_pin"), icon: "pin", onselect: () => chatStore.setPinned(chat.id, !chat.pinned) });
    list.push({ label: chat.is_muted ? $t("msg_chat_unmute") : $t("msg_chat_mute"), icon: chat.is_muted ? "bell" : "bell-off", onselect: () => chatStore.setMuted(chat.id, !chat.is_muted) });
    list.push({ label: chat.archived ? $t("msg_chat_unarchive") : $t("msg_chat_archive"), icon: "archive", onselect: () => chatStore.setArchived(chat.id, !chat.archived) });
    // A group is left, not deleted: its own entries say how.
    if (!isGroup) list.push({ label: $t("msg_chat_delete"), icon: "trash-2", danger: true, onselect: async () => { if (await confirmStore.ask($t("msg_chat_delete_confirm", { name: chat.title }), $t("msg_chat_delete"), true)) { await chatStore.deleteChat(chat.id); onback?.(); } } });
    return list;
  });

  async function guard(fn: () => Promise<unknown>) {
    error = '';
    try { await fn(); } catch (e) { error = explain(e); }
  }

  async function send(text: string) {
    error = '';
    try {
      if (editing) {
        const id = editing.id;
        editing = null;
        await chatStore.edit(id, text);
      } else {
        const r = replyTo?.id;
        replyTo = null;
        atBottom = true;
        await chatStore.send(text, r);
      }
    } catch (e) {
      error = explain(e);
      throw e;
    }
  }

  async function record(rec: MessengerRecording) {
    error = "";
    atBottom = true;
    try { await chatStore.sendRecording(rec); }
    catch (e) { error = explain(e); throw e; }
  }

  function editLast() {
    const mine = [...chatStore.messages].reverse().find((m) => m.direction === 'out' && m.content_type === 'text' && !m.deleted);
    if (mine) { replyTo = null; editing = mine; }
  }

  async function attach(paths: string[]) {
    error = "";
    atBottom = true;
    // Picked together, shown together.
    const batch = paths.length > 1 ? crypto.randomUUID() : undefined;
    for (const p of paths) {
      try { await chatStore.sendFile(p, undefined, batch); }
      catch (e) { error = explain(e); break; }
    }
  }

  function openMenu(e: MouseEvent, m: MessengerMessage) {
    e.preventDefault();
    menu = { open: true, x: e.clientX, y: e.clientY, m };
  }

  async function jumpTo(id: string) {
    const el = scroller?.querySelector(`[data-mid="${id}"]`);
    if (!el) return;
    el.scrollIntoView({ block: 'center', behavior: 'smooth' });
    highlighted = id;
    setTimeout(() => { if (highlighted === id) highlighted = null; }, 1400);
  }

  const items = $derived.by((): MenuEntry[] => {
    const m = menu.m;
    if (!m) return [];
    const own = m.direction === 'out';
    const list: MenuEntry[] = [];
    if (!m.deleted) {
      if (chat.can_send) list.push({ label: $t('msg_message_reply'), icon: 'reply', onselect: () => { editing = null; replyTo = m; } });
      if (m.text) list.push({ label: $t('msg_copy'), icon: 'copy', onselect: () => navigator.clipboard.writeText(m.text ?? '').catch(() => {}) });
      if (own && m.content_type === 'text' && chat.can_send) list.push({ label: $t('msg_message_edit'), icon: 'pencil', onselect: () => { replyTo = null; editing = m; } });
      if (own && m.status === 'failed') list.push({ label: $t('msg_message_retry'), icon: 'refresh-cw', onselect: () => guard(() => chatStore.retry(m.id)) });
      list.push({ type: 'separator' });
      list.push({ label: $t('msg_message_delete_me'), icon: 'trash-2', danger: true, onselect: () => guard(() => chatStore.remove(m.id, false)) });
      if (own || canModerate?.(m)) list.push({ label: $t("msg_message_delete_all"), icon: "trash-2", danger: true, onselect: () => guard(() => chatStore.remove(m.id, true)) });
    }
    return list;
  });

  const systemLine = (m: MessengerMessage) => systemText?.(m) ?? $t(`msg_sys_${m.text}` as "msg_sys_request_sent", { name: chat.title });
</script>

{#snippet attachment(m: MessengerMessage)}
  <MediaBubble message={m} />
{/snippet}

{#snippet bubble(item: Extract<TimelineItem, { type: "bubble" }>)}
  <MessageBubble message={item.message} first={item.first} last={item.last} showAuthor={item.showAuthor} peerTitle={chat.title} {author} highlighted={highlighted === item.message.id}
    onmenu={openMenu} onreplyclick={jumpTo} onretry={(m) => guard(() => chatStore.retry(m.id))} media={attachment} />
{/snippet}

{#snippet album(item: Extract<TimelineItem, { type: "album" }>)}
  {@const who = item.messages[0].sender_pubkey}
  <AlbumBubble messages={item.messages} variant={item.variant} first={item.first} last={item.last} author={item.showAuthor && author ? author(who) : null} authorTint={tint(who)} {highlighted}
    onmenu={openMenu} onretry={(m) => guard(() => chatStore.retry(m.id))} />
{/snippet}

{#snippet composerTools()}
  <AttachButton disabled={!sessionActive || !canAttach || !!editing} onfiles={attach} />
{/snippet}

<section class="window">
  <header class="head" class:phone>
    {#if onback}
      {#if phone}<button class="icon back" onclick={onback} aria-label={$t('msg_back')}><Icon name="chevron-left" size={24} /></button>
      {:else}<button class="icon back narrow-only" onclick={onback} title={$t('msg_back')}><Icon name="arrow-left" size={16} /></button>{/if}
    {/if}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="ident" class:clickable={!!ontitle} onclick={() => ontitle?.()}>
    <Avatar url={chat.picture} label={chat.title} seed={chat.peer_pubkey ?? chat.id} size={36} />
    <div class="who">
      <div class="title">{chat.title}{#if chat.is_muted}<span class="dim"><Icon name="bell-off" size={12} /></span>{/if}</div>
      <div class="sub">
        {#if !sessionActive}{$t('msg_chat_locked')}
        {:else if !online}<span class="offline">{$t('msg_chat_offline')}</span>
        {:else if subtitle}{@render subtitle()}
        {:else}<code>{chat.peer_npub ? `${chat.peer_npub.slice(0, 14)}…${chat.peer_npub.slice(-6)}` : ''}</code>{/if}
      </div>
    </div>
    </div>
    {#if actions}{@render actions()}{/if}
    <button class="icon" onclick={openChatMenu} title={$t("msg_chat_menu")}><Icon name="more-vertical" size={16} /></button>
  </header>

  {#if !isGroup}<RelationBanner {chat} busy={acting} onaction={act} />{/if}
  {#if banner}{@render banner()}{/if}

  <div class="scroll" bind:this={scroller} {onscroll}>
    <div class="content" bind:this={content}>
    {#if chatStore.loadingOlder}<div class="loading"><Icon name="loader" size={14} /></div>{/if}
    {#if chatStore.loading && chatStore.messages.length === 0}
      <div class="placeholder">{$t('loading')}</div>
    {:else if chatStore.messages.length === 0}
      <div class="placeholder">
        <Icon name="lock" size={22} />
        <p>{$t('msg_chat_empty')}</p>
        <span>{$t('msg_chat_empty_hint')}</span>
      </div>
    {:else}
      <Timeline messages={chatStore.messages} many={isGroup} systemText={systemLine} {bubble} {album} />
    {/if}
    </div>
  </div>

  {#if !atBottom}
    <button class="to-bottom" onclick={() => { atBottom = true; scrollToBottom(); }} title={$t('msg_chat_to_bottom')}>
      <Icon name="chevrons-down" size={16} />
    </button>
  {/if}

  {#if error}<div class="error-line">{error}</div>{/if}

  {#if chat.can_send}
    <Composer {replyTo} {editing} peerTitle={chat.title} disabled={!sessionActive} draftKey={chat.id} autofocus={!phone} oneditlast={editLast} onrecording={record} canRecord={canAttach}
      oncancel={() => { replyTo = null; editing = null; }} onsend={send} tools={composerTools} />
  {:else if footer}
    {@render footer()}
  {:else}
    <div class="no-composer"><Icon name="lock" size={13} />{$t("msg_chat_cannot_send")}</div>
  {/if}
</section>

<MediaViewer />
<ContextMenu bind:open={menu.open} x={menu.x} y={menu.y} {items} onclose={() => (menu.open = false)} />
<ContextMenu bind:open={chatMenu.open} x={chatMenu.x} y={chatMenu.y} items={chatItems} onclose={() => (chatMenu.open = false)} />

<style>
  .window { position: relative; display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--bg); }
  .head { display: flex; align-items: center; gap: var(--sp-3); padding: var(--sp-2) var(--sp-4); border-bottom: 1px solid var(--border); background: var(--surface); flex-shrink: 0; min-height: 56px; }
  /* The phone: the bar colour of the shell, the strip under the status bar included; its back is the chevron of every phone screen. */
  .head.phone { background: var(--m-nav, var(--surface)); padding-left: var(--sp-1); }
  .head.phone .back { color: var(--text); padding: 10px; }
  .ident { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; flex: 1; }
  .ident.clickable { cursor: pointer; }
  .who { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .title { display: flex; align-items: center; gap: 6px; font-weight: var(--fw-bold); font-size: var(--fs-base); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dim { color: var(--text-3); display: inline-flex; }
  .sub { font-size: var(--fs-2xs); color: var(--text-3); }
  .sub code { font-family: var(--font-mono); }
  .offline { color: var(--warn-text); }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .icon:hover { color: var(--text); background: var(--surface-3); }
  @media (pointer: coarse) {
    .icon { padding: 10px; }
    .head { padding-inline: var(--sp-2); gap: var(--sp-2); }
    .to-bottom { width: 44px; height: 44px; }
  }
  .narrow-only { display: none; }
  @media (max-width: 860px) { .narrow-only { display: inline-flex; } }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-3) 0; display: flex; flex-direction: column; }
  .content { margin-top: auto; display: flex; flex-direction: column; flex-shrink: 0; }
  /* Nothing to show yet: the note stands in the middle. */
  .content:has(> .placeholder) { margin-bottom: auto; }
  .placeholder { margin: auto; display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); color: var(--text-3); text-align: center; padding: var(--sp-6); }
  .placeholder p { margin: 0; color: var(--text-body); font-weight: var(--fw-semibold); font-size: var(--fs-sm); }
  .placeholder span { font-size: var(--fs-xs); max-width: 320px; line-height: 1.5; }
  .loading { display: flex; justify-content: center; color: var(--text-3); padding: var(--sp-2); }
  .to-bottom {
    position: absolute; right: var(--sp-4); bottom: 84px; width: 36px; height: 36px; border-radius: 50%;
    border: 1px solid var(--border); background: var(--surface); color: var(--text-2); cursor: pointer;
    display: inline-flex; align-items: center; justify-content: center; box-shadow: var(--shadow);
  }
  .to-bottom:hover { color: var(--text); }
  .error-line { padding: 6px var(--sp-4); font-size: var(--fs-xs); color: var(--danger-text); background: var(--danger-bg); border-top: 1px solid var(--danger-border); }
  .no-composer { display: flex; align-items: center; justify-content: center; gap: 6px; padding: var(--sp-3); border-top: 1px solid var(--border); background: var(--surface); font-size: var(--fs-xs); color: var(--text-3); }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { tick, untrack, type Snippet } from 'svelte';
  import { t, locale, localeTag } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import MessageBubble from './MessageBubble.svelte';
  import AlbumBubble from './AlbumBubble.svelte';
  import QuickReactions from './QuickReactions.svelte';
  import { reactionTarget } from './quick-reactions';
  import { canEdit, editTarget } from './editable';
  import Timeline from '../content/Timeline.svelte';
  import type { TimelineItem } from '../content/types';
  import { tint } from '../shared/tint';
  import Composer from './Composer.svelte';
  import RelationBanner from './RelationBanner.svelte';
  import MediaBubble from '../media/MediaBubble.svelte';
  import AttachButton from '../media/AttachButton.svelte';
  import MediaViewer from '../media/MediaViewer.svelte';
  import { viewer } from '../media/viewer.svelte';
  import type { MessengerPicked, MessengerPoster, MessengerRecording } from '../api';
  import { posterUrl, videoPoster } from '../media/poster';
  import { chatStore } from '../chats/chatStore.svelte';
  import { messengerStore } from '../store.svelte';
  import { confirmStore } from '../shared/confirm.svelte';
  import { onKeyboard } from '../shared/keyboard';
  import { onPhone } from '../shared/phone';
  import { nameStore } from '../groups/names.svelte';
  import { presenceStore } from '../presence/presenceStore.svelte';
  import { dmErrorCode, MAX_SEND_BYTES, mediaErrorCode, messengerApi, type DmAction, type MessengerChat, type MessengerMessage } from '../api';
  import { mediaErrorText, tooLargeName } from '../media/errors';
  import { bytes } from '../shared/format';
  import TransfersChip from './TransfersChip.svelte';

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
  /**
   * `m`: the message pressed, what the items act on. `target`: what a reaction from the menu lands on (an album's last part).
   * `caption`: what Edit changes (an album's part that carries its caption).
   */
  let menu = $state<{ open: boolean; x: number; y: number; m: MessengerMessage | null; target: MessengerMessage | null; caption: MessengerMessage | null }>({ open: false, x: 0, y: 0, m: null, target: null, caption: null });
  /** The menu of a message shows the whole emoji picker instead of the quick strip. */
  let menuMore = $state(false);
  let chatMenu = $state<{ open: boolean; x: number; y: number }>({ open: false, x: 0, y: 0 });
  /** Who in a group read a message of mine: opened from its menu, where that menu was. */
  let seen = $state<{ open: boolean; x: number; y: number; who: string[] }>({ open: false, x: 0, y: 0, who: [] });
  let acting = $state(false);

  // A promise, as the dot: shown as connected until the runtime takes it
  // back after a while without relays.
  const link = $derived(messengerStore.status?.runtime?.link ?? 'ok');
  const sessionActive = $derived(!!messengerStore.status?.runtime?.session_active);
  // A DM's peer: online now, or when last seen; nothing for groups or when unknown.
  const presence = $derived(chat.kind === 'dm' ? presenceStore.status(chat.peer_pubkey) : null);
  const presenceText = $derived(chat.kind === 'dm' ? presenceStore.label(chat.peer_pubkey) : null);

  // Leaving a chat drops reply/edit state, and a picture it had open.
  let lastChat = '';
  $effect(() => {
    if (chat.id !== lastChat) { lastChat = chat.id; replyTo = null; editing = null; error = ''; atBottom = true; picked = []; original = false; viewer.close(); }
  });

  // The list of transfers asked for a message of this chat: shown once the
  // chat has it, older pages read until it does.
  $effect(() => {
    const want = chatStore.jumpTo;
    if (!want || want.chatId !== chat.id || chatStore.loading || chatStore.activeId !== chat.id) return;
    chatStore.jumpTo = null;
    untrack(() => chatStore.reach(want.messageId))
      .then((shown) => {
        if (!shown || chatStore.activeId !== want.chatId) return;
        atBottom = false;
        tick().then(() => jumpTo(want.messageId, false));
      })
      .catch(() => {});
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

  // On the phone the composer is glass over the conversation: the messages
  // end above it and pass under it when scrolled.
  let bottomH = $state(0);
  $effect(() => { void bottomH; if (atBottom) tick().then(scrollToBottom); });

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
    return mediaErrorText(e, (key) => $t(key));
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
    // An archived chat is closed (as a deleted one): its page is left with it.
    list.push({ label: chat.archived ? $t("msg_chat_unarchive") : $t("msg_chat_archive"), icon: "archive", onselect: async () => { const archive = !chat.archived; await chatStore.setArchived(chat.id, archive); if (archive) onback?.(); } });
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
      } else if (picked.length) {
        await sendPicked(text);
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
    const mine = [...chatStore.messages].reverse().find((m) => canEdit(m, chat.can_send) && (m.content_type === 'text' || !!m.text));
    if (mine) { replyTo = null; editing = mine; }
  }

  /**
   * Picked files wait above the field until they are sent; `file` is `null` while it is read in.
   * A video carries a frame of it (`poster`): the tray shows it, and so does the other side before the file is there.
   */
  let picked = $state<{ key: number; file: MessengerPicked | null; poster?: MessengerPoster | null }[]>([]);
  let pickSeq = 0;
  const attached = $derived(picked.length === 0 ? null : picked.some((p) => !p.file) ? 'loading' : 'ready');
  /** Pictures of this sending go as they are, not compressed (the owner's per-send switch). */
  let original = $state(false);
  const pictures = $derived(picked.some((p) => p.file?.kind === 'image'));

  /** The name of a picked path, for a note before the file is read: `content://…/IMG_1.jpg` too. */
  function pathName(path: string): string {
    const last = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
    try { return decodeURIComponent(last); } catch { return last; }
  }

  async function attach(paths: string[]) {
    error = "";
    const refused: string[] = [];
    const other: string[] = [];
    for (const p of paths) {
      const key = ++pickSeq;
      picked = [...picked, { key, file: null }];
      try {
        const file = await messengerApi.media.importPicked(p);
        // The runtime refuses a file over the limit; one that told no size then is caught here.
        if (file.size > MAX_SEND_BYTES) throw new Error(`err.file_too_large: ${file.name}`);
        picked = picked.map((x) => (x.key === key ? { key, file } : x));
        if (file.kind === 'video' && file.url) {
          const poster = await videoPoster(file.url);
          if (poster) picked = picked.map((x) => (x.key === key ? { key, file: { ...file, preview: posterUrl(poster) }, poster } : x));
        }
      } catch (e) {
        picked = picked.filter((x) => x.key !== key);
        // A `content://` path names nothing a person reads: the refusal carries the file's name.
        if (mediaErrorCode(e) === 'err.file_too_large') refused.push($t('msg_xfer_too_big', { name: tooLargeName(e) ?? pathName(p) }));
        else other.push(explain(e));
      }
    }
    error = [...refused, ...other].join('\n');
  }

  function unpick(key: number) {
    picked = picked.filter((x) => x.key !== key);
    if (!picked.length) original = false;
  }

  /**
   * Picked together, sent together: one album, the text as its caption.
   * The caption goes with the last file, so the list of chats shows it.
   * With a reply the text goes on its own after the files, as the reply.
   */
  async function sendPicked(text: string) {
    const files = picked.flatMap((x) => (x.file ? [{ ...x.file, poster: x.poster ?? null }] : []));
    const asIs = original;
    picked = [];
    original = false;
    atBottom = true;
    const reply = replyTo?.id;
    replyTo = null;
    const batch = files.length > 1 ? crypto.randomUUID() : undefined;
    for (const [i, f] of files.entries()) {
      try {
        await chatStore.sendFile(f.path, !reply && i === files.length - 1 ? text : undefined, batch, asIs, f.poster);
      } catch (e) {
        // What did not leave waits again.
        picked = files.slice(i).map(({ poster, ...file }) => ({ key: ++pickSeq, file, poster }));
        original = asIs;
        throw e;
      }
    }
    if (reply && text) await chatStore.send(text, reply);
  }

  function openMenu(e: MouseEvent, m: MessengerMessage, album?: MessengerMessage[]) {
    e.preventDefault();
    menuMore = false;
    menu = { open: true, x: e.clientX, y: e.clientY, m, target: reactionTarget(m, album), caption: editTarget(m, album) };
  }

  /** A message one may react to: it is there, it left this device, and this chat takes words from me. */
  const reactable = (m: MessengerMessage | null): m is MessengerMessage =>
    !!m && !m.deleted && chat.can_send && m.content_type !== 'system' && !m.id.startsWith('local:') && m.status !== 'failed';

  function react(m: MessengerMessage, emoji: string) {
    menu.open = false;
    guard(() => chatStore.react(m.id, emoji));
  }

  function showMore() {
    menuMore = true;
    // The menu grows: a new place makes it measure itself and stay on screen.
    menu.y -= 0.01;
  }

  /** `smooth`: a jump within what is on screen; a chat just opened lands at once, before its pictures grow. */
  async function jumpTo(id: string, smooth = true) {
    const el = scroller?.querySelector(`[data-mid="${id}"]`);
    if (!el) return;
    el.scrollIntoView({ block: 'center', behavior: smooth ? 'smooth' : 'auto' });
    highlighted = id;
    setTimeout(() => { if (highlighted === id) highlighted = null; }, 1400);
  }

  const items = $derived.by((): MenuEntry[] => {
    const m = menu.m;
    if (!m) return [];
    const own = m.direction === 'out';
    const list: MenuEntry[] = [];
    if (!m.deleted) {
      // An upload on its way is swapped for a message with another id: a quote of it would point at nothing.
      if (chat.can_send && !m.id.startsWith('local:') && m.status !== 'uploading' && m.status !== 'paused') list.push({ label: $t('msg_message_reply'), icon: 'reply', onselect: () => { editing = null; replyTo = m; } });
      if (m.text) list.push({ label: $t('msg_copy'), icon: 'copy', onselect: () => navigator.clipboard.writeText(m.text ?? '').catch(() => {}) });
      const caption = menu.caption ?? m;
      if (canEdit(caption, chat.can_send)) list.push({ label: $t('msg_message_edit'), icon: 'pencil', onselect: () => { replyTo = null; editing = caption; } });
      if (own && m.status === 'failed') list.push({ label: $t('msg_message_retry'), icon: 'refresh-cw', onselect: () => guard(() => chatStore.retry(m.id)) });
      if (own && isGroup && m.seen_by?.length) {
        const { x, y } = menu;
        list.push({ label: $t('msg_seen_by', { n: String(m.seen_by.length) }), icon: 'eye', onselect: () => (seen = { open: true, x, y, who: [...m.seen_by] }) });
      }
      list.push({ type: 'separator' });
      list.push({ label: $t('msg_message_delete_me'), icon: 'trash-2', danger: true, onselect: () => guard(() => chatStore.remove(m.id, false)) });
      if (own || canModerate?.(m)) list.push({ label: $t("msg_message_delete_all"), icon: "trash-2", danger: true, onselect: () => guard(() => chatStore.remove(m.id, true)) });
    }
    return list;
  });

  const systemLine = (m: MessengerMessage) => systemText?.(m) ?? $t(`msg_sys_${m.text}` as "msg_sys_request_sent", { name: chat.title });
</script>

{#snippet readers()}
  <div class="readers">
    {#each seen.who as pk (pk)}
      <div class="reader"><Avatar url={nameStore.picture(pk)} label={nameStore.label(pk)} seed={pk} size={24} /><span>{nameStore.label(pk)}</span></div>
    {:else}
      <div class="reader">{$t('msg_seen_by_none')}</div>
    {/each}
  </div>
{/snippet}

{#snippet quick()}
  {#if menu.target}<QuickReactions message={menu.target} more={menuMore} onmore={showMore} onreact={(e) => menu.target && react(menu.target, e)} />{/if}
{/snippet}

{#snippet attachment(m: MessengerMessage)}
  <MediaBubble message={m} />
{/snippet}

{#snippet bubble(item: Extract<TimelineItem, { type: "bubble" }>)}
  <MessageBubble message={item.message} first={item.first} last={item.last} showAuthor={item.showAuthor} peerTitle={chat.title} {author} highlighted={highlighted === item.message.id}
    onmenu={openMenu} onreplyclick={jumpTo} onretry={(m) => guard(() => chatStore.retry(m.id))} media={attachment} onreacterror={(e) => (error = explain(e))} />
{/snippet}

{#snippet album(item: Extract<TimelineItem, { type: "album" }>)}
  {@const who = item.messages[0].sender_pubkey}
  <AlbumBubble messages={item.messages} variant={item.variant} first={item.first} last={item.last} author={item.showAuthor && author ? author(who) : null} authorTint={tint(who)} {highlighted}
    onmenu={(e, m) => openMenu(e, m, item.messages)} onretry={(m) => guard(() => chatStore.retry(m.id))} onreacterror={(e) => (error = explain(e))} />
{/snippet}

{#snippet composerTools()}
  <AttachButton disabled={!sessionActive || !canAttach || !!editing} onfiles={attach} />
{/snippet}

{#snippet tray()}
  <div class="tray">
    {#each picked as p (p.key)}
      <div class="pick" class:picture={!!p.file?.preview} title={p.file ? `${p.file.name} · ${bytes(p.file.size, localeTag($locale))}` : undefined}>
        {#if p.file?.preview}<img src={p.file.preview} alt={p.file.name} />
        {:else if p.file}<Icon name={p.file.kind === "video" ? "video" : p.file.kind === "image" ? "image" : "file"} size={20} /><span class="pick-name">{p.file.name}</span>
        {:else}<Icon name="loader" size={18} />{/if}
        {#if p.file}<span class="pick-size">{bytes(p.file.size, localeTag($locale))}</span>{/if}
        <button class="unpick" tabindex="-1" onpointerdown={(e) => e.preventDefault()} onmousedown={(e) => e.preventDefault()} onclick={() => unpick(p.key)}
          aria-label={$t("msg_attach_remove")} title={$t("msg_attach_remove")}><Icon name="x" size={12} /></button>
      </div>
    {/each}
  </div>
  {#if pictures}
    <button class="original" class:on={original} role="switch" aria-checked={original} title={$t('msg_xfer_original_hint')}
      onpointerdown={(e) => e.preventDefault()} onmousedown={(e) => e.preventDefault()} onclick={() => (original = !original)}>
      <span class="box" aria-hidden="true">{#if original}<Icon name="check" size={11} />{/if}</span>{$t('msg_xfer_original')}
    </button>
  {/if}
{/snippet}

<section class="window" style:--bottom-h={phone ? `${bottomH}px` : undefined}>
  <header class="head" class:phone>
    {#if onback}
      {#if phone}<button class="icon back" onclick={onback} aria-label={$t('msg_back')}><Icon name="chevron-left" size={24} /></button>
      {:else}<button class="icon back narrow-only" onclick={onback} title={$t('msg_back')}><Icon name="arrow-left" size={16} /></button>{/if}
    {/if}
    {#snippet identity()}
      <Avatar url={chat.picture} label={chat.title} seed={chat.peer_pubkey ?? chat.id} size={36} online={presence === 'online'} />
      <div class="who">
        <div class="title">{chat.title}{#if chat.is_muted}<span class="dim"><Icon name="bell-off" size={12} /></span>{/if}</div>
        <div class="sub">
          {#if !sessionActive}{$t('msg_chat_locked')}
          {:else if link === 'lost'}<span class="offline">{$t('msg_chat_offline')}</span>
          {:else if link === 'waiting'}<span class="offline">{$t('msg_chat_connecting')}</span>
          {:else if subtitle}{@render subtitle()}
          {:else if presenceText}<span class:online={presence === 'online'}>{presenceText}</span>
          {:else}<code>{chat.peer_npub ? `${chat.peer_npub.slice(0, 14)}…${chat.peer_npub.slice(-6)}` : ''}</code>{/if}
        </div>
      </div>
    {/snippet}
    <!-- The face and the name open the panel about the chat (the header has no other button for it);
         a key opens it too. -->
    {#if ontitle}
      <div class="ident clickable" role="button" tabindex="0" onclick={() => ontitle?.()}
        onkeydown={(e) => { if ((e.key === 'Enter' || e.key === ' ') && e.target === e.currentTarget) { e.preventDefault(); ontitle?.(); } }}>
        {@render identity()}
      </div>
    {:else}
      <div class="ident">{@render identity()}</div>
    {/if}
    <TransfersChip {phone} />
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

  <div class="bottom" class:float={phone} bind:clientHeight={bottomH}>
  {#if error}<div class="error-line">{error}</div>{/if}

  {#if chat.can_send}
    <Composer {replyTo} {editing} peerTitle={chat.title} disabled={!sessionActive} draftKey={chat.id} autofocus={!phone} oneditlast={editLast} onrecording={record} canRecord={canAttach}
      oncancel={() => { replyTo = null; editing = null; }} onsend={send} tools={composerTools} {attached} attachments={picked.length ? tray : undefined} />
  {:else if footer}
    {@render footer()}
  {:else}
    <div class="no-composer"><Icon name="lock" size={13} />{$t("msg_chat_cannot_send")}</div>
  {/if}
  </div>
</section>

<MediaViewer />
<ContextMenu bind:open={menu.open} x={menu.x} y={menu.y} {items} header={reactable(menu.target) ? quick : undefined} onclose={() => (menu.open = false)} />
<ContextMenu bind:open={seen.open} x={seen.x} y={seen.y} items={[]} header={readers} onclose={() => (seen.open = false)} />
<ContextMenu bind:open={chatMenu.open} x={chatMenu.x} y={chatMenu.y} items={chatItems} onclose={() => (chatMenu.open = false)} />

<style>
  .window { position: relative; display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--bg); }
  .head { display: flex; align-items: center; gap: var(--sp-3); padding: var(--sp-2) var(--sp-4); border-bottom: 1px solid var(--border); background: var(--surface); flex-shrink: 0; min-height: 56px; }
  /* The phone: the bar colour of the shell, the strip under the status bar included; its back is the chevron of every phone screen. */
  .head.phone { background: var(--m-nav, var(--surface)); padding-left: var(--sp-1); }
  .head.phone .back { color: var(--text); padding: 10px; }
  .ident { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; flex: 1; }
  .ident.clickable { cursor: pointer; border-radius: var(--radius-sm); }
  .ident.clickable:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .who { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .title { display: flex; align-items: center; gap: 6px; font-weight: var(--fw-bold); font-size: var(--fs-base); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dim { color: var(--text-3); display: inline-flex; }
  .sub { font-size: var(--fs-2xs); color: var(--text-3); }
  .sub code { font-family: var(--font-mono); }
  .offline { color: var(--warn-text); }
  .sub .online { color: var(--accent); }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .icon:hover { color: var(--text); background: var(--surface-3); }
  @media (pointer: coarse) {
    .icon { padding: 10px; }
    .head { padding-inline: var(--sp-2); gap: var(--sp-2); }
    .to-bottom { width: 44px; height: 44px; }
  }
  .narrow-only { display: none; }
  @media (max-width: 860px) { .narrow-only { display: inline-flex; } }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-3) 0 calc(var(--sp-3) + var(--bottom-h, 0px)); display: flex; flex-direction: column; }
  .bottom { flex-shrink: 0; }
  .bottom.float { position: absolute; left: 0; right: 0; bottom: 0; }
  .content { margin-top: auto; display: flex; flex-direction: column; flex-shrink: 0; }
  /* Nothing to show yet: the note stands in the middle. */
  .content:has(> .placeholder) { margin-bottom: auto; }
  .placeholder { margin: auto; display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); color: var(--text-3); text-align: center; padding: var(--sp-6); }
  .placeholder p { margin: 0; color: var(--text-body); font-weight: var(--fw-semibold); font-size: var(--fs-sm); }
  .placeholder span { font-size: var(--fs-xs); max-width: 320px; line-height: 1.5; }
  .loading { display: flex; justify-content: center; color: var(--text-3); padding: var(--sp-2); }
  .to-bottom {
    position: absolute; right: var(--sp-4); bottom: max(84px, calc(var(--bottom-h, 0px) + var(--sp-4))); width: 36px; height: 36px; border-radius: 50%;
    border: 1px solid var(--border); background: var(--surface); color: var(--text-2); cursor: pointer;
    display: inline-flex; align-items: center; justify-content: center; box-shadow: var(--shadow);
  }
  .to-bottom:hover { color: var(--text); }
  /* Picked files above the field: pictures as they look, the rest by name. */
  .tray { display: flex; gap: var(--sp-2); overflow-x: auto; padding: 2px 0; scrollbar-width: thin; }
  .pick {
    position: relative; flex-shrink: 0; width: 64px; height: 64px; border-radius: var(--radius-md); overflow: hidden;
    display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 4px; padding: 4px;
    background: var(--surface-2); border: 1px solid var(--border); color: var(--text-2);
  }
  .pick.picture { padding: 0; }
  .pick img { width: 100%; height: 100%; object-fit: cover; }
  .pick-name { max-width: 100%; font-size: var(--fs-2xs); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pick-size {
    position: absolute; left: 3px; bottom: 3px; max-width: calc(100% - 6px); padding: 0 4px; border-radius: var(--radius-pill);
    font-size: 9px; line-height: 15px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    background: rgba(0, 0, 0, 0.55); color: #fff; pointer-events: none;
  }
  .pick:not(.picture) .pick-name { margin-bottom: 12px; }
  /* Pictures go compressed unless this is on, for this sending only. */
  .original {
    display: inline-flex; align-items: center; gap: 6px; align-self: flex-start; margin-top: 2px; padding: 4px 8px 4px 6px; cursor: pointer;
    border: 1px solid var(--border); border-radius: var(--radius-pill); background: var(--surface-2); color: var(--text-2);
    font: inherit; font-size: var(--fs-2xs); font-weight: var(--fw-semibold);
  }
  .original.on { border-color: var(--accent-tint-border); background: var(--accent-tint); color: var(--accent-text-2); }
  .original .box {
    width: 14px; height: 14px; border-radius: 4px; border: 1.5px solid currentColor; display: inline-flex; align-items: center; justify-content: center;
  }
  .error-line { white-space: pre-line; }
  .unpick {
    position: absolute; top: 3px; right: 3px; width: 20px; height: 20px; border: none; border-radius: 50%; cursor: pointer;
    display: inline-flex; align-items: center; justify-content: center; background: rgba(0, 0, 0, 0.55); color: #fff;
  }
  @media (pointer: coarse) { .pick { width: 72px; height: 72px; } .unpick { width: 24px; height: 24px; } }
  .error-line { padding: 6px var(--sp-4); font-size: var(--fs-xs); color: var(--danger-text); background: var(--danger-bg); border-top: 1px solid var(--danger-border); }
  .readers { display: flex; flex-direction: column; gap: var(--sp-1); max-height: 50vh; overflow-y: auto; }
  .reader { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; color: var(--text); font-size: var(--fs-sm); }
  .reader span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .no-composer { display: flex; align-items: center; justify-content: center; gap: 6px; padding: var(--sp-3); border-top: 1px solid var(--border); background: var(--surface); font-size: var(--fs-xs); color: var(--text-3); }
</style>

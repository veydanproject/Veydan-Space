<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { Snippet } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { clock } from '../shared/time';
  import MessageContent from '../content/MessageContent.svelte';
  import ContactCardMessage from '../content/ContactCardMessage.svelte';
  import { longpress } from '../shared/longpress';
  import { tint } from '../shared/tint';
  import { mediaOf, type MessengerMessage } from '../api';
  import { replyLine, replyThumb } from '../push/wording';
  import { chatStore } from '../chats/chatStore.svelte';
  import { logSendFailure, sendFailureKey } from './send-failure';
  import { lateAt, shownStatus } from './delivery';
  import Reactions from './Reactions.svelte';

  interface Props {
    message: MessengerMessage;
    /** First and last bubble of a run from the same sender: the corners between bubbles of a run are small. */
    first: boolean;
    last?: boolean;
    peerTitle: string;
    /** Chats of many: the name is shown above this message. */
    showAuthor?: boolean;
    /** Chats of many: name of whoever wrote a message. */
    author?: (pubkey: string) => string;
    highlighted?: boolean;
    onmenu: (e: MouseEvent, m: MessengerMessage) => void;
    onreplyclick: (id: string) => void;
    onretry: (m: MessengerMessage) => void;
    /** Renders the attachment of a `media` message (stage 6). */
    media?: Snippet<[MessengerMessage]>;
    /** A reaction was refused: the window explains why. */
    onreacterror?: (e: unknown) => void;
  }
  let { message: m, first, last = true, peerTitle, showAuthor = false, author, highlighted = false, onmenu, onreplyclick, onretry, media, onreacterror }: Props = $props();

  // The backend's own words go to the log; the Retry button says it in the UI's language.
  $effect(() => {
    if (m.status === 'failed') logSendFailure(m.id, m.failure_reason);
  });

  const out = $derived(m.direction === 'out');
  // A card has no text: a reply to one names the person, when the card is in the loaded window.
  const replyCard = $derived.by(() => {
    const r = m.reply_to;
    if (!r || r.text) return null;
    const card = chatStore.messages.find((x) => x.id === r.id)?.card;
    return card ? card.label : null;
  });
  // A circle is its own shape: no bubble under it.
  const bare = $derived(!m.deleted && !m.reply_to && !m.text && m.content_type === 'media' && mediaOf(m)?.kind === 'circle');
  // A message on its way is shown as sent for a moment; one timer moves it
  // to the clock when that moment is over.
  let now = $state(Date.now() / 1000);
  $effect(() => {
    if (m.status !== 'queued') return;
    const at = Date.now() / 1000;
    now = at;
    const wait = lateAt(m) - at;
    if (wait <= 0) return;
    const timer = setTimeout(() => (now = Date.now() / 1000), wait * 1000 + 50);
    return () => clearTimeout(timer);
  });
  const shown = $derived(shownStatus(m, now));
  const statusIcon = $derived(
    shown === 'sent' ? 'check' : shown === 'delivered' || shown === 'read' ? 'check-check' : shown === 'failed' ? 'alert-triangle' : shown === 'uploading' ? 'upload' : 'clock',
  );
  const statusTitle = $derived($t(`msg_status_${shown}` as 'msg_status_sent'));
</script>

<div class="line" class:out class:first class:last class:highlighted data-mid={m.id}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="bubble" class:bare class:deleted={m.deleted} class:failed={m.status === 'failed'} oncontextmenu={(e) => onmenu(e, m)}
    use:longpress={{ onpress: (p) => onmenu(new MouseEvent('contextmenu', { clientX: p.x, clientY: p.y }), m) }}>
    {#if author && showAuthor}<span class="author" style="color: {tint(m.sender_pubkey)}">{author(m.sender_pubkey)}</span>{/if}
    {#if m.reply_to && !m.deleted}
      {@const thumb = replyThumb(m.reply_to)}
      <button class="reply" onclick={() => onreplyclick(m.reply_to!.id)}>
        {#if thumb}<img class="reply-thumb" src={thumb} alt="" aria-hidden="true" />{/if}
        <span class="reply-body">
          <span class="reply-who">{author ? author(m.reply_to.sender_pubkey) : m.reply_to.sender_pubkey === m.sender_pubkey && out || m.reply_to.sender_pubkey !== m.sender_pubkey && !out ? $t("msg_you") : peerTitle}</span>
          <span class="reply-text">{#if m.reply_to.text && !m.reply_to.deleted}<MessageContent text={m.reply_to.text} plain />{:else}{replyLine(m.reply_to, replyCard, $t)}{/if}</span>
        </span>
      </button>
    {/if}

    {#if m.deleted}
      <span class="tomb"><Icon name="ban" size={12} /> {$t('msg_message_deleted')}</span>
    {:else if m.content_type === 'contact'}
      <ContactCardMessage message={m} />
    {:else if m.content_type === 'media' && media}
      {@render media(m)}
      {#if m.text}<MessageContent text={m.text} />{/if}
    {:else if m.text}
      <MessageContent text={m.text} />
    {:else}
      <span class="tomb">{$t('msg_message_unsupported', { type: m.content_type })}</span>
    {/if}

    {#if !m.deleted && m.reactions?.length}<Reactions message={m} onerror={onreacterror} />{/if}

    <span class="meta">
      {#if m.edited_at && !m.deleted}<span>{$t('msg_message_edited')}</span>{/if}
      <span>{clock(m.created_at)}</span>
      {#if out && !m.deleted}
        <span class="status {shown}" title={statusTitle}><Icon name={statusIcon} size={12} /></span>
      {/if}
    </span>
  </div>
  {#if m.status === 'failed' && out && !m.deleted && !m.id.startsWith('local:')}
    <button class="retry" onclick={() => onretry(m)} title={$t(sendFailureKey(m.failure_reason))}>
      <Icon name="refresh-cw" size={12} />{$t('msg_message_retry')}
    </button>
  {/if}
</div>

<style>
  .line { display: flex; flex-direction: column; align-items: flex-start; padding: 1px var(--sp-4); border-radius: var(--radius-sm); transition: background 0.6s var(--ease); }
  .line.out { align-items: flex-end; }
  .line.first { margin-top: var(--sp-2); }
  .line.highlighted { background: var(--accent-tint); }
  .bubble {
    position: relative; max-width: min(620px, 78%); padding: 7px 11px 6px;
    background: var(--surface-2); color: var(--text); border: 1px solid var(--border);
    --r: 14px; --r-joined: 5px;
    border-radius: var(--r); display: flex; flex-direction: column; gap: 3px; min-width: 64px;
  }
  /* A run of one author: the corners that touch the next bubble are small. */
  .line:not(.first) .bubble { border-top-left-radius: var(--r-joined); }
  .line:not(.last) .bubble { border-bottom-left-radius: var(--r-joined); }
  .line.out .bubble { background: var(--accent-tint); border-color: var(--accent-tint-border); border-top-left-radius: var(--r); border-bottom-left-radius: var(--r); }
  .line.out:not(.first) .bubble { border-top-right-radius: var(--r-joined); }
  .line.out:not(.last) .bubble { border-bottom-right-radius: var(--r-joined); }
  .bubble.failed { border-color: var(--danger-border); }
  .line .bubble.bare, .line.out .bubble.bare { background: none; border-color: transparent; padding: 0; }
  /* Touch: a long press opens the menu, so it must not start a selection. */
  @media (pointer: coarse) {
    .bubble { max-width: 86%; -webkit-touch-callout: none; }
    .line { padding-inline: var(--sp-3); }
  }
  .author { font-size: var(--fs-2xs); font-weight: var(--fw-bold); line-height: 1.2; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 260px; }
  .tomb { display: inline-flex; align-items: center; gap: 5px; font-size: var(--fs-xs); color: var(--text-3); font-style: italic; }
  .meta { display: inline-flex; align-items: center; gap: 5px; align-self: flex-end; font-size: var(--fs-2xs); color: var(--text-3); line-height: 1; }
  .status { display: inline-flex; }
  .status.sent, .status.delivered { color: var(--accent-text-2); }
  .status.read { color: var(--accent); }
  .status.failed { color: var(--danger-text); }
  .reply {
    display: flex; align-items: center; gap: 8px; text-align: left; width: 100%;
    border: none; border-left: 2px solid var(--accent); border-radius: 4px; background: var(--surface-3);
    padding: 3px 8px; color: inherit; font: inherit; cursor: pointer; min-width: 0;
  }
  .line.out .reply { background: color-mix(in srgb, var(--accent) 10%, transparent); }
  .reply-body { display: flex; flex-direction: column; gap: 1px; min-width: 0; flex: 1; }
  .reply-thumb { flex: 0 0 auto; width: 36px; height: 36px; object-fit: cover; border-radius: 4px; background: var(--surface-3); }
  .reply-who { font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--accent-text-2); }
  .reply-text { font-size: var(--fs-xs); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 420px; }
  .reply-text :global(.text) { font-size: inherit; line-height: inherit; white-space: inherit; user-select: none; }
  .retry {
    display: inline-flex; align-items: center; gap: 4px; margin-top: 2px; border: none; background: none;
    color: var(--danger-text); font: inherit; font-size: var(--fs-2xs); cursor: pointer; padding: 2px 4px;
  }
  .retry:hover { text-decoration: underline; }
</style>

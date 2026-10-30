<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { untrack, type Snippet } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { clock } from '../shared/time';
  import MessageContent from '../content/MessageContent.svelte';
  import ContactCardMessage from '../content/ContactCardMessage.svelte';
  import BotCardMessage from '../content/BotCardMessage.svelte';
  import { longpress } from '../shared/longpress';
  import { swipeReply } from '../shared/swipe-reply';
  import { tint } from '../shared/tint';
  import { mediaOf, type MessengerMessage } from '../api';
  import { replyLine, replyThumb } from '../push/wording';
  import { chatStore } from '../chats/chatStore.svelte';
  import { logSendFailure, sendFailureKey } from './send-failure';
  import { lateAt, shownStatus } from './delivery';
  import Reactions from './Reactions.svelte';
  import HoverActions from './HoverActions.svelte';

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
    /** Answer it (a swipe to the left on a touch screen); absent where it cannot be answered. */
    onreply?: () => void;
    /** A computer: the buttons of a hovered message; `react` absent where one cannot react. */
    actions?: { react?: (anchor: HTMLElement) => void; forward?: () => void; more: (anchor: HTMLElement) => void };
    /** Its menu is open. */
    selected?: boolean;
  }
  let { message: m, first, last = true, peerTitle, showAuthor = false, author, highlighted = false, onmenu, onreplyclick, onretry, media, onreacterror, onreply, actions, selected = false }: Props = $props();

  // The backend's own words go to the log; the Retry button says it in the UI's language.
  $effect(() => {
    if (m.status === 'failed') logSendFailure(m.id, m.failure_reason);
  });

  const out = $derived(m.direction === 'out');
  /** It came while the chat was open (not from the history): it rises in. */
  const fresh = untrack(() => Date.now() / 1000 - m.created_at < 4);
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
  // The time sits on the last line of the text, as in other messengers, when
  // text ends the bubble: no card instead of it.
  const metaInline = $derived(
    !!m.text && !m.deleted && m.content_type !== 'contact' && !m.bot_card
      && (m.content_type !== 'media' || !!media),
  );
</script>

<div class="line" class:out class:first class:last class:highlighted class:selected class:fresh data-mid={m.id} use:swipeReply={{ onreply: m.deleted ? undefined : onreply }}>
  <div class="brow">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="bubble" class:bare class:contact={m.content_type === 'contact' && !m.deleted} class:deleted={m.deleted} class:failed={m.status === 'failed'} oncontextmenu={(e) => onmenu(e, m)}
    use:longpress={{ onpress: (p) => onmenu(new MouseEvent('contextmenu', { clientX: p.x, clientY: p.y }), m) }}>
    {#if author && showAuthor}<span class="author" style="color: {tint(m.sender_pubkey)}">{author(m.sender_pubkey)}</span>{/if}
    {#if m.fwd && !m.deleted}<span class="fwd">{m.fwd.name ? $t('msg_fwd_from', { name: m.fwd.name }) : $t('msg_fwd_from_nameless')}</span>{/if}
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
      <span class="tomb"><Icon name="ban" size={14} />{$t('msg_message_deleted')}</span>
    {:else if m.content_type === 'contact'}
      <ContactCardMessage message={m} />
    {:else if m.bot_card}
      <!-- Only what the runtime calls a card of the bot is one; the same words from anyone else are a message. -->
      <BotCardMessage card={m.bot_card} id={m.id} at={m.created_at} />
    {:else if m.content_type === 'media' && media}
      {@render media(m)}
      {#if m.text}<div class="body"><MessageContent text={m.text} rich={m.rich} />{#if metaInline}{@render meta()}{/if}</div>{/if}
    {:else if m.text}
      <div class="body"><MessageContent text={m.text} rich={m.rich} />{#if metaInline}{@render meta()}{/if}</div>
    {:else}
      <span class="tomb">{$t('msg_message_unsupported', { type: m.content_type })}</span>
    {/if}


    {#snippet meta()}
    <span class="meta" class:inline={metaInline}>
      <!-- Waiting for a moderator: only its author and the moderators have it at all. -->
      {#if m.review === 'pending' && !m.deleted}<span class="pending"><Icon name="clock" size={12} />{$t('msg_mod_pending')}</span>{/if}
      <!-- Turned down by a moderator: its author alone is shown it, and told so. -->
      {#if m.review === 'rejected' && !m.deleted}<span class="pending rejected"><Icon name="x" size={12} />{$t('msg_mod_log_reject')}</span>{/if}
      {#if m.edited_at && !m.deleted}<span>{$t('msg_message_edited')}</span>{/if}
      <span>{clock(m.created_at)}</span>
      {#if out && !m.deleted}
        <span class="status {shown}" title={statusTitle}>{#key statusIcon}<Icon name={statusIcon} size={14} />{/key}</span>
      {/if}
    </span>
    {/snippet}
    {#if !metaInline}{@render meta()}{/if}
  </div>
    {#if actions && !m.deleted}<HoverActions {onreply} {actions} {selected} />{/if}
  </div>
  <!-- Reactions hang under the bubble, on its side. -->
  {#if !m.deleted && m.reactions?.length}<div class="reacts"><Reactions message={m} onerror={onreacterror} onadd={actions?.react} /></div>{/if}
  {#if m.status === 'failed' && out && !m.deleted && !m.id.startsWith('local:')}
    <!-- Why it did not go, said in the row (it was the button's hint), and the way to send it again (mockups c1.js). -->
    <div class="failed-row">
      <Icon name="alert-triangle" size={14} /><span class="failed-why">{$t(sendFailureKey(m.failure_reason))}</span>
      <button class="btn btn-ghost btn-sm retry" onclick={() => onretry(m)} title={$t(sendFailureKey(m.failure_reason))}>
        <Icon name="refresh-cw" size={16} />{$t('msg_message_retry')}
      </button>
    </div>
  {/if}
</div>

<style>

  /* A computer: the actions of the hovered message beside its bubble (after it, before it for mine). */
  .brow { display: flex; align-items: center; gap: var(--sp-1); width: 100%; min-width: 0; }
  .line.out .brow { flex-direction: row-reverse; }
  /* The message whose menu is open. */
  .line.selected { background: var(--accent-bg); }
  .line.fresh { animation: msg-in var(--dur-base) var(--ease); }
  @keyframes msg-in { from { opacity: 0; transform: translateY(8px); } }
  @media (prefers-reduced-motion: reduce) { .line.fresh { animation: none; } }
  .line { position: relative; display: flex; flex-direction: column; align-items: flex-start; padding: 1px var(--sp-4); border-radius: var(--radius-sm); transition: background 0.6s var(--ease); }
  .line.out { align-items: flex-end; }
  /* Runs of one author are groups (mockups c7.js): 2 between the bubbles of a run (the lines' own padding), 16 between runs. */
  .line.first { margin-top: calc(var(--sp-4) - 2px); }
  .line.highlighted { background: var(--accent-tint); }
  .bubble {
    position: relative; max-width: min(620px, 86%); padding: 6px var(--sp-3);
    background: var(--bubble-in-bg); color: var(--text); border: 1px solid var(--bubble-in-border);
    --r: var(--radius-head); --r-joined: var(--radius-badge); --r-tail: var(--radius-badge);
    border-radius: var(--r); display: flex; flex-direction: column; gap: 2px; min-width: 64px;
  }
  /* A run of one author: the corners that touch the next bubble are small. */
  .line:not(.first) .bubble { border-top-left-radius: var(--r-joined); }
  /* The corner on the author's side at the bottom is always the small one: it points at the author. */
  .line .bubble { border-bottom-left-radius: var(--r-tail); }
  .line.out .bubble { background: var(--bubble-out-bg); border-color: var(--bubble-out-border); color: var(--bubble-out-text); border-top-left-radius: var(--r); border-bottom-left-radius: var(--r); }
  .line.out:not(.first) .bubble { border-top-right-radius: var(--r-joined); }
  .line.out .bubble { border-bottom-right-radius: var(--r-tail); }
  .bubble.failed { border-color: var(--danger-border); }
  /* A contact's card: the bubble itself is the card, with the card's padding. */
  .bubble.contact { padding: var(--sp-3) var(--sp-3) var(--sp-2); }
  .bubble.contact > .meta { margin-top: var(--sp-1); }
  .line .bubble.bare, .line.out .bubble.bare { background: none; border-color: transparent; padding: 0; }
  /* Touch: a long press opens the menu, so it must not start a selection. */
  @media (pointer: coarse) {
    .bubble { max-width: 86%; -webkit-touch-callout: none; }
    .line { padding-inline: var(--sp-3); touch-action: pan-y; }
  }
  /* Forwarded: whose it was, over what it says. */
  .fwd { font-size: var(--fs-2xs); font-style: italic; color: var(--text-2); line-height: 1.3; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 320px; }
  .line.out .fwd { color: var(--bubble-out-meta); }
  .author { font-size: var(--fs-sm); font-weight: var(--fw-bold); line-height: 1.2; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 260px; }
  .tomb { display: inline-flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-base); color: var(--text-3); font-style: italic; }
  .meta { display: inline-flex; align-items: center; gap: var(--sp-1); align-self: flex-end; font-size: var(--fs-2xs); font-variant-numeric: tabular-nums; color: var(--text-2); line-height: 1; white-space: nowrap; }
  /* On the last line of the text: a float after the words takes the room left
     on that line, or drops to a line of its own at the right when there is none. */
  .reacts { margin-top: var(--sp-1); max-width: min(620px, 86%); }
  .body { display: block; }
  .body::after { content: ''; display: block; clear: both; }
  .meta.inline {
    float: right; margin-left: var(--sp-3); box-sizing: border-box;
    height: calc(var(--fs-body) * var(--lh-body)); align-items: flex-end; padding-bottom: var(--sp-1);
  }
  .line.out .meta { color: var(--bubble-out-meta); }
  .status { display: inline-flex; }
  .pending {
    display: inline-flex; align-items: center; gap: 3px; height: 18px; padding: 0 6px; border-radius: var(--radius-pill);
    background: var(--warn-bg); color: var(--warn-text); font-weight: var(--fw-semibold);
  }
  .pending.rejected { background: var(--danger-bg); color: var(--danger-text); }
  .status.sent, .status.delivered { color: var(--accent-text-2); }
  .status.read { color: var(--accent-text); }
  .status.failed { color: var(--danger-text); }
  .status.waiting, .status.paused { color: var(--warn-text); }
  .status :global(svg) { animation: status-in var(--dur-fast) var(--ease); }
  .status.read :global(svg) { animation: status-pop var(--dur-fast) var(--ease); }
  @keyframes status-in { from { opacity: 0; } }
  @keyframes status-pop { from { opacity: 0; transform: scale(0.8); } }
  @media (prefers-reduced-motion: reduce) { .status :global(svg) { animation: none !important; } }
  .reply {
    display: flex; align-items: center; gap: 8px; text-align: left; width: 100%;
    border: none; border-left: 2px solid var(--accent); border-radius: var(--radius-xs); background: var(--surface-3);
    padding: 3px 8px; color: inherit; font: inherit; cursor: pointer; min-width: 0;
  }
  .line.out .reply { background: color-mix(in srgb, var(--accent) 10%, transparent); }
  .reply-body { display: flex; flex-direction: column; gap: 1px; min-width: 0; flex: 1; }
  .reply-thumb { flex: 0 0 auto; width: 36px; height: 36px; object-fit: cover; border-radius: var(--radius-xs); background: var(--surface-3); }
  .reply-who { font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--accent-text-2); }
  .reply-text { font-size: var(--fs-xs); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 420px; }
  .reply-text :global(.text) { font-size: inherit; line-height: inherit; white-space: inherit; user-select: none; }
  .failed-row { display: flex; align-items: center; flex-wrap: wrap; justify-content: flex-end; gap: var(--sp-2); margin-top: var(--sp-1); max-width: min(620px, 86%); font-size: var(--fs-sm); color: var(--danger-text); }
  .failed-row > :global(svg) { flex: none; }
  .failed-why { min-width: 0; }
  .retry { gap: var(--sp-1); color: var(--text); }
</style>

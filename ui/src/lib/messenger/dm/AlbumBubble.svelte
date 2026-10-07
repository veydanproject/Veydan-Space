<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Attachments sent by one action, in one bubble: pictures and videos as a
  mosaic, files as a list. A picture sent alone is drawn here too, as a
  picture. Each attachment stays a message of its own: the menu, the
  status and the transfer belong to the one that was pressed. Reactions
  belong to the album: they are kept on its last part (`reactionTarget`),
  whichever part the menu was opened on, and shown under the whole.
-->
<script lang="ts">
  import { countKey, locale, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import MediaBubble from '../media/MediaBubble.svelte';
  import { transferStore } from '../media/transferStore.svelte';
  import MessageContent from '../content/MessageContent.svelte';
  import { mosaic, rowHeight } from '../content/mosaic';
  import { mediaFamily } from '../content/timeline';
  import type { AlbumVariant } from '../content/types';
  import { clock } from '../shared/time';
  import { longpress } from '../shared/longpress';
  import type { MessengerMessage } from '../api';
  import { logSendFailure, sendFailureKey } from './send-failure';
  import { lateAt, shownStatus, worstOf } from './delivery';
  import Reactions from './Reactions.svelte';
  import { reactionTarget } from './quick-reactions';

  interface Props {
    messages: MessengerMessage[];
    variant: AlbumVariant;
    first: boolean;
    last: boolean;
    /** Chats of many: name of whoever wrote. */
    author?: string | null;
    authorTint?: string;
    highlighted?: string | null;
    onmenu: (e: MouseEvent, m: MessengerMessage) => void;
    onretry: (m: MessengerMessage) => void;
    /** A reaction was refused: the window explains why. */
    onreacterror?: (e: unknown) => void;
  }
  let { messages, variant, first, last, author = null, authorTint = 'inherit', highlighted = null, onmenu, onretry, onreacterror }: Props = $props();

  const head = $derived(messages[0]);
  const tail = $derived(messages[messages.length - 1]);
  /** The part the album's reactions are kept on. */
  const reacted = $derived(reactionTarget(head, messages));
  const out = $derived(head.direction === 'out');
  const captions = $derived(messages.filter((m) => m.text?.trim()));
  const failed = $derived(messages.filter((m) => m.status === 'failed'));
  // The backend's own words go to the log; the button says it in the UI's language.
  $effect(() => {
    for (const f of failed) logSendFailure(f.id, f.failure_reason);
  });
  const edited = $derived(messages.some((m) => m.edited_at));
  const pictures = $derived(messages.filter((m) => mediaFamily(m) === 'visual'));
  const files = $derived(messages.filter((m) => mediaFamily(m) !== 'visual'));
  const rows = $derived(mosaic(pictures));
  const fileRows = $derived(mosaic(files));
  /** Pictures without words: the time sits on the last picture. */
  const overlay = $derived(variant === 'visual' && captions.length === 0);
  // Parts on their way are shown as sent for a moment (`delivery`); one
  // timer moves the album to the clock when the last of them is late.
  let now = $state(Date.now() / 1000);
  const late = $derived(Math.max(0, ...messages.filter((m) => m.status === 'queued').map(lateAt)));
  $effect(() => {
    if (!late) return;
    const at = Date.now() / 1000;
    now = at;
    if (late <= at) return;
    const timer = setTimeout(() => (now = Date.now() / 1000), (late - at) * 1000 + 50);
    return () => clearTimeout(timer);
  });
  /** The worst of what the parts are in. */
  const status = $derived(worstOf(messages.map((m) => shownStatus(m, now))));
  const statusIcon = $derived(status === 'sent' ? 'check' : status === 'delivered' || status === 'read' ? 'check-check' : status === 'failed' ? 'alert-triangle' : status === 'uploading' ? 'upload' : 'clock');

  /** A part on its way up: its placeholder says so, or its transfer does. */
  const sending = (m: MessengerMessage) => {
    const live = transferStore.get(m.id);
    return m.status === 'uploading' || (live?.direction === 'up' && ['queued', 'running', 'waiting_retry'].includes(live.status));
  };
  /** Not stored yet: on its way, paused, or failed before it left. */
  const notYet = (m: MessengerMessage) => sending(m) || m.status === 'paused' || (m.status === 'failed' && m.id.startsWith('local:'));
  /** While parts of an album of mine go up: how many of them are there already. */
  const uploaded = $derived(out && messages.length > 1 && messages.some(sending) ? { k: messages.filter((m) => !notYet(m)).length, n: messages.length } : null);

  const press = (m: MessengerMessage) => ({
    onpress: (p: { x: number; y: number }) => onmenu(new MouseEvent('contextmenu', { clientX: p.x, clientY: p.y }), m),
  });
</script>

{#snippet meta(onPicture: boolean)}
  <span class="meta" class:on-picture={onPicture}>
    {#if edited}<span>{$t('msg_message_edited')}</span>{/if}
    <span>{clock(tail.created_at)}</span>
    {#if out}<span class="status {status}" title={$t(`msg_status_${status}` as 'msg_status_sent')}><Icon name={statusIcon} size={12} /></span>{/if}
  </span>
{/snippet}

<div class="line" class:out class:first class:last class:highlighted={messages.some((m) => m.id === highlighted)}>
  <div class="bubble {variant}" class:failed={failed.length > 0} class:bare={overlay && !author}>
    {#if author}<span class="author" style="color: {authorTint}">{author}</span>{/if}

    {#if pictures.length}
      <div class="mosaic" aria-label={messages.length > 1 ? $t('msg_album_n', { n: String(messages.length) }) : undefined}>
        {#each rows as row, r (r)}
          {@const h = rowHeight(row.length, rows.length)}
          <div class="row" style={h ? `height:${h}px` : ''}>
            {#each row as m (m.id)}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="cell" data-mid={m.id} oncontextmenu={(e) => onmenu(e, m)} use:longpress={press(m)}>
                <MediaBubble message={m} variant="tile" fit={h ? 'cover' : 'natural'} />
              </div>
            {/each}
          </div>
        {/each}
        {#if overlay}{@render meta(true)}{/if}
      </div>
    {/if}
    {#if files.length}
      <div class="cards" class:under={pictures.length > 0}>
        {#each fileRows as row, r (r)}
          <div class="card-row">
            {#each row as m (m.id)}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="card-cell" data-mid={m.id} oncontextmenu={(e) => onmenu(e, m)} use:longpress={press(m)}>
                <MediaBubble message={m} variant="card" wide={row.length === 1} />
              </div>
            {/each}
          </div>
        {/each}
      </div>
    {/if}

    {#if uploaded}
      <div class="uploaded" aria-live="polite">
        <Icon name="upload" size={11} />{$t(countKey('msg_xfer_album', uploaded.n, $locale), { k: String(uploaded.k), n: String(uploaded.n) })}
      </div>
    {/if}
    {#if !reacted.deleted && reacted.reactions?.length}<div class="reactions"><Reactions message={reacted} onerror={onreacterror} /></div>{/if}
    {#each captions as m (m.id)}<div class="caption"><MessageContent text={m.text ?? ''} /></div>{/each}
    {#if !overlay}{@render meta(false)}{/if}
  </div>
  {#each failed.filter((m) => out && !m.deleted && !m.id.startsWith('local:')) as m (m.id)}
    <button class="retry" onclick={() => onretry(m)} title={$t(sendFailureKey(m.failure_reason))}>
      <Icon name="refresh-cw" size={12} />{$t('msg_message_retry')}
    </button>
  {/each}
</div>

<style>
  .line { display: flex; flex-direction: column; align-items: flex-start; padding: 1px var(--sp-4); border-radius: var(--radius-sm); transition: background 0.6s var(--ease); }
  .line.out { align-items: flex-end; }
  .line.first { margin-top: var(--sp-2); }
  .line.highlighted { background: var(--accent-tint); }
  .bubble {
    --r: 14px; --r-joined: 5px;
    max-width: min(420px, 78%); background: var(--surface-2); color: var(--text); border: 1px solid var(--border);
    border-radius: var(--r); display: flex; flex-direction: column; gap: 4px; overflow: hidden;
  }
  /* Every album is as wide as the mosaic it holds, pictures or files. */
  .bubble.visual, .bubble.files, .bubble.mixed { width: min(420px, 78%); padding: 3px; }
  /* A run of one author: the corners that touch the next bubble are small. */
  .line:not(.first) .bubble { border-top-left-radius: var(--r-joined); }
  .line:not(.last) .bubble { border-bottom-left-radius: var(--r-joined); }
  .line.out .bubble { background: var(--accent-tint); border-color: var(--accent-tint-border); border-top-left-radius: var(--r); border-bottom-left-radius: var(--r); }
  .line.out:not(.first) .bubble { border-top-right-radius: var(--r-joined); }
  .line.out:not(.last) .bubble { border-bottom-right-radius: var(--r-joined); }
  .bubble.failed { border-color: var(--danger-border); }
  .author { font-size: var(--fs-2xs); font-weight: var(--fw-bold); line-height: 1.2; padding: 4px 8px 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .mosaic { position: relative; display: flex; flex-direction: column; gap: 2px; border-radius: calc(var(--r) - 3px); overflow: hidden; }
  .row { display: flex; gap: 2px; min-height: 0; }
  .cell { flex: 1 1 0; min-width: 0; display: flex; }

  .cards { display: flex; flex-direction: column; gap: 3px; }
  .card-row { display: flex; gap: 3px; }
  .card-cell { flex: 1 1 0; min-width: 0; display: flex; }
  .card-cell > :global(*) { flex: 1; }

  .caption { padding: 0 8px; }
  .uploaded { display: flex; align-items: center; gap: 4px; padding: 0 8px; font-size: var(--fs-2xs); color: var(--text-3); }
  .reactions { padding: 2px 5px 0; }
  .meta { display: inline-flex; align-items: center; gap: 5px; align-self: flex-end; font-size: var(--fs-2xs); color: var(--text-3); line-height: 1; padding: 0 8px 5px; }
  .meta.on-picture {
    position: absolute; right: 6px; bottom: 6px; padding: 3px 7px; border-radius: var(--radius-pill);
    background: rgba(0, 0, 0, 0.5); color: #fff; pointer-events: none;
  }
  .status { display: inline-flex; }
  .status.sent, .status.delivered { color: var(--accent-text-2); }
  .status.read { color: var(--accent); }
  .meta.on-picture .status { color: #fff; }
  .status.failed { color: var(--danger-text); }
  .retry {
    display: inline-flex; align-items: center; gap: 4px; margin-top: 2px; border: none; background: none;
    color: var(--danger-text); font: inherit; font-size: var(--fs-2xs); cursor: pointer; padding: 2px 4px;
  }
  .retry:hover { text-decoration: underline; }
  @media (pointer: coarse) {
    .bubble { -webkit-touch-callout: none; }
    .bubble.visual, .bubble.files, .bubble.mixed { width: 86%; max-width: 86%; }
    .line { padding-inline: var(--sp-3); }
  }
</style>

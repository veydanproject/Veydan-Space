<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Attachments sent by one action, in one bubble: pictures and videos as a
  mosaic, files as a list. A picture sent alone is drawn here too, as a
  picture. Each attachment stays a message of its own: the menu, the
  status and the transfer belong to the one that was pressed.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import MediaBubble from '../media/MediaBubble.svelte';
  import MessageContent from '../content/MessageContent.svelte';
  import { mosaic, rowHeight } from '../content/mosaic';
  import { mediaFamily } from '../content/timeline';
  import type { AlbumVariant } from '../content/types';
  import { clock } from '../shared/time';
  import { longpress } from '../shared/longpress';
  import type { MessengerMessage } from '../api';
  import { logSendFailure, sendFailureKey } from './send-failure';

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
  }
  let { messages, variant, first, last, author = null, authorTint = 'inherit', highlighted = null, onmenu, onretry }: Props = $props();

  const head = $derived(messages[0]);
  const tail = $derived(messages[messages.length - 1]);
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
  /** The worst of what the parts are in. */
  const status = $derived(
    failed.length ? 'failed' : messages.some((m) => m.status === 'uploading') ? 'uploading'
      : messages.some((m) => m.status === 'queued' || m.status === 'paused') ? 'queued' : 'sent',
  );
  const statusIcon = $derived(status === 'sent' ? 'check' : status === 'failed' ? 'alert-triangle' : status === 'uploading' ? 'upload' : 'clock');

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

    {#each captions as m (m.id)}<div class="caption"><MessageContent text={m.text ?? ''} /></div>{/each}
    {#if !overlay}{@render meta(false)}{/if}
  </div>
  {#each failed.filter((m) => out && !m.id.startsWith('local:')) as m (m.id)}
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
  .meta { display: inline-flex; align-items: center; gap: 5px; align-self: flex-end; font-size: var(--fs-2xs); color: var(--text-3); line-height: 1; padding: 0 8px 5px; }
  .meta.on-picture {
    position: absolute; right: 6px; bottom: 6px; padding: 3px 7px; border-radius: var(--radius-pill);
    background: rgba(0, 0, 0, 0.5); color: #fff; pointer-events: none;
  }
  .status { display: inline-flex; }
  .status.sent { color: var(--accent-text-2); }
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

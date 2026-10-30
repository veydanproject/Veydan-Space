<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One post of the feed, the post on its own page, or one comment of a thread.

  A post reads as an article (the redesign of 2026-10-23): its first line is
  its title, then its pictures and videos as a mosaic, its files as a list,
  the rest of its words (three lines in the feed, «Читать» when there is
  more), the reactions, and at its foot who wrote it and when, edited when,
  and the way to its comments. `full`: the post page, all of it.

  A comment keeps its look: who wrote it and when on top, its words, its
  file, the reactions and «Ответить».

  What a moderator sees and others do not (a post that waits for a check,
  one hidden) is marked. The menu is the window's: a right click, a long
  press, or the «more» button.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { Avatar, MessageContent, clock, listStamp, longpress, mediaTitle, mosaic, onPhone, rowHeight, stamp } from '$lib/messenger/seams/shared';
  import ChannelMedia from './ChannelMedia.svelte';
  import { isLocal } from './feedStore.svelte';
  import { albumParts, mediaFamily } from './albums';
  import { nameStore } from './names.svelte';
  import { isRecording, mediaOf, type ChannelDetail, type FeedItemView } from './api';
  import { recordingCard } from './post';
  import { staffOf } from './permissions';
  import { cutAt, dropFirstLine, editedAt, postTitle } from './post';
  import { undrawn, watchSize } from './sizes';

  /** How many lines of words a card of the feed shows under the title. */
  const LINES = 3;

  interface Props {
    item: FeedItemView;
    detail: ChannelDetail;

    /** A comment: smaller, and with the name of the comment it answers. */
    comment?: boolean;
    replyTo?: string | null;
    /** The post page: the whole post, its title larger, the role of its author, no way to its comments (they follow). */
    full?: boolean;
    /** The posts of an album, oldest first (`item` is the last, with the caption, the reactions and the comments). */
    parts?: FeedItemView[] | null;
    /** Mine, on its way: written here and not told of by the runtime yet (or waiting in its outbox). */
    pending?: boolean;
    /** Mine, given up by the outbox: why; with a way to send it again. */
    failure?: string | null;
    onretry?: (item: FeedItemView) => void;
    highlighted?: boolean;
    /** Comments shown under a post: its button; absent, none. */
    oncomments?: (item: FeedItemView) => void;
    /** «Читать»: the post page, for a post whose words do not fit in the card. */
    onread?: (item: FeedItemView) => void;
    onreact: (item: FeedItemView, emoji: string) => void;
    onmenu: (at: { x: number; y: number }, item: FeedItemView) => void;
    /** «Ответить» under a comment. */
    onreply?: (item: FeedItemView) => void;
    /** The title of the post's section, when it is not everyone's (communities-wire §14.3). */
    section?: string | null;
    /** The section's look (stage 4 of the redesign): its colour (any CSS colour) and its icon; none, the accent and a folder. */
    sectionColor?: string | null;
    sectionIcon?: string | null;
    /** The post editor's preview: the post as its readers will see it, with no menu (its items are `local:`). */
    preview?: boolean;
    /** Files picked and not sent yet, by the id of their item in the preview: what the tray shows of each (a picture's URL, or none). */
    pictures?: Record<string, string | null> | null;
  }
  let {
    item, detail, comment = false, replyTo = null, full = false, highlighted = false, oncomments, onread, onreact, onmenu, onreply, section = null,
    sectionColor = null, sectionIcon = null, pending = false, failure = null, onretry, parts = null, preview = false, pictures = null,
  }: Props = $props();
  /** What a file of the preview is drawn from: the tray's, for a file not sent yet. */
  const pickedOf = (p: FeedItemView) => (pictures ? { src: pictures[p.id] ?? null } : null);

  const phone = onPhone();

  /** An album: pictures as a mosaic, other files as a list. */
  const album = $derived(parts && parts.length > 1 ? albumParts(parts) : null);
  const rows = $derived(album ? mosaic(album.visual) : []);
  /** The posts whose words are shown: of an album, those with words (the last as a rule); else the post. */
  const words = $derived(parts && parts.length > 1 ? parts.filter((p) => !p.deleted && p.text) : item.text ? [item] : []);

  /** The title: the first line of the first words; what follows it, and the words of any other part, are the text. */
  const head = $derived(words.length ? postTitle(words[0].text, words[0].fmt) : { title: '', rest: '' });
  const title = $derived(comment || item.deleted ? '' : head.title);
  /** The rest of the first words, as MessageContent draws them: Markdown loses the title's line of its blocks. */
  const firstRest = $derived.by(() => {
    const w = words[0];
    if (!w) return null;
    const rich = w.rich?.length ? (head.title ? dropFirstLine(w.rich) : w.rich) : null;
    if (rich ? !rich.length : !head.rest) return null;
    return { id: w.id, text: head.rest, rich };
  });
  const restParts = $derived([
    ...(firstRest ? [firstRest] : []),
    ...words.slice(1).map((w) => ({ id: w.id, text: w.text ?? '', rich: w.rich ?? null })),
  ]);

  /** A single file post: shown where an album shows its pictures (a picture, a video) or its files (the rest). */
  const single = $derived(!album && item.media_json && !item.deleted ? item : null);
  const singleVisual = $derived(single ? mediaFamily(single) === 'visual' : false);
  /** No title over pictures: the menu sits on their corner. */
  const menuOnMedia = $derived(!comment && !title && !item.deleted && (singleVisual || (album?.visual.length ?? 0) > 0));
  /** A voice message or a circle alone (no words, so no title): the player first, the menu beside it, no empty row above. */
  const menuBeside = $derived(recordingCard({ comment, title, deleted: item.deleted, recording: !!single && isRecording(mediaOf(single)), section, pinned: item.pinned }));

  /** Written here, before the command answered with its id, or the preview of the post editor: nothing can be done with it (yet). */
  const local = $derived(isLocal(item.id) || preview);

  const role = $derived(staffOf(detail, item.author).role);
  const name = $derived(nameStore.label(item.author));
  const edited = $derived(editedAt(item));
  /** The comments of a post open from its foot; not of one a moderator has yet to let through. */
  const commentable = $derived(!!oncomments && !full && !item.deleted && !local && failure === null && item.review !== 'pending' && item.review !== 'rejected');

  // ── Three lines in the feed, «Читать» when the words (or the title) do not fit ──
  let titleEl = $state<HTMLElement | null>(null);
  let restInner = $state<HTMLElement | null>(null);
  let overflow = $state(false);
  /** The words cut after their last line shown: the height kept, and that line (where «…» stands). */
  let cut = $state<{ height: number; top: number; bottom: number; right: number; edge: boolean } | null>(null);
  /** Measured once at least: a text of no more lines than shown keeps its height, whatever its blocks add. */
  let measured = $state(false);
  $effect(() => {
    const t0 = titleEl;
    const inner = restInner;
    if (full || comment || (!t0 && !inner)) { overflow = false; cut = null; return; }
    const measure = () => {
      // A card of a list kept hidden keeps what it measured: shown again at the same width, nothing changes.
      if (undrawn((inner ?? t0)!)) return;
      const c = inner ? cutAt(inner, LINES) : null;
      const width = inner?.clientWidth ?? 0;
      // A line that runs to the edge fades out under «…»; a shorter one has «…» after its last word.
      const next = c ? { height: c.height, top: c.line.top, bottom: c.line.bottom, right: Math.min(c.line.right, width), edge: c.line.right > width - 28 } : null;
      if (JSON.stringify(next) !== JSON.stringify(cut)) cut = next;
      measured = !!inner;
      overflow = !!c || (!!t0 && t0.scrollHeight > t0.clientHeight + 1);
    };
    // Out of the effect: what it reads of `cut` is not a reason to run it again.
    untrack(measure);
    // The words and the title, not the box: its height is the cut itself. One observer for all the cards (sizes.ts).
    return watchSize([t0, inner].filter((el): el is HTMLElement => !!el), measure);
  });

  function menuAt(e: MouseEvent) {
    e.preventDefault();
    if (local) return;
    onmenu({ x: e.clientX, y: e.clientY }, item);
  }

  function moreAt(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    onmenu({ x: r.right, y: r.bottom + 4 }, item);
  }

  /** What a file is called for the screen reader in a post without words: its name; a voice message or a circle, by its kind. */
  const mediaName = $derived.by(() => {
    const m = mediaOf(item);
    return m ? (isRecording(m) ? mediaTitle(m.kind, m.name, $t) : m.name) : '';
  });
</script>

{#snippet more(over: boolean)}
  {#if !local}
    <button class="more" class:over onclick={moreAt} aria-label={$t('chan_post_menu')} title={$t('chan_post_menu')}>
      <Icon name="more-vertical" size={18} />
    </button>
  {/if}
{/snippet}

{#snippet sectionMark()}
  <span class="scope-mark" class:tinted={!!sectionColor} style:--tag={sectionColor} title={$t('chan_section')}>
    <Icon name={sectionIcon ?? 'folder'} size={14} /><span class="scope-name">{section}</span>
  </span>
{/snippet}

{#snippet marks()}
  {#if item.review === 'pending'}<div class="mark warn"><Icon name="clock" size={14} />{$t('chan_review_pending')}</div>{/if}
  {#if item.review === 'rejected'}<div class="mark danger"><Icon name="x" size={14} />{$t('chan_review_rejected')}</div>{/if}
  {#if item.hidden}<div class="mark"><Icon name="eye-off" size={14} />{$t('chan_hidden_mark')}</div>{/if}
  {#if failure !== null}
    <div class="mark danger unsent-mark" role="alert" title={failure || undefined}>
      <Icon name="alert-triangle" size={14} />{$t('chan_not_sent')}
      {#if onretry}<button class="retry" onclick={() => onretry?.(item)}><Icon name="refresh-cw" size={11} />{$t('chan_retry')}</button>{/if}
    </div>
  {/if}
{/snippet}

{#snippet reactions()}
  {#if !item.deleted && !local && item.reactions.length}
    <div class="reactions msg-font">
      {#each item.reactions as r (r.emoji)}
        <button class="react" class:mine={r.mine} aria-pressed={r.mine} onclick={() => onreact(item, r.emoji)} title={$t('chan_react')}>
          <span class="emoji">{r.emoji}</span><span class="count">{r.count}</span>
        </button>
      {/each}
    </div>
  {/if}
{/snippet}

<article class="post" class:comment class:card={!comment && !full} class:full class:phone class:preview class:highlighted class:pending={item.review === 'pending'}
  class:hidden={item.hidden} class:sending={pending && !failure} class:unsent={failure !== null}
  id={full ? undefined : `post-${item.id}`} oncontextmenu={menuAt} use:longpress={{ onpress: (p) => { if (!local) onmenu(p, item); } }}>
  {#if comment}
    <header class="head">
      <Avatar url={nameStore.picture(item.author)} label={name} seed={item.author} size={32} />
      <div class="who">
        <span class="name">
          <span class="name-text">{name}</span>
          {#if role !== 'member'}<span class="role {role}">{$t(`chan_role_${role}` as 'chan_role_admin')}</span>{/if}
        </span>
        <span class="when" title={stamp(item.created_at)}>
          <!-- A separator of its own: Svelte drops the space a block starts with. -->
          {clock(item.created_at)}{#if replyTo}<span class="sep" aria-hidden="true">·</span><Icon name="reply" size={11} />{replyTo}{/if}
          {#if edited}<span class="sep" aria-hidden="true">·</span><span class="edited" title={$t('chan_edited_full', { when: stamp(edited) })}>{$t('chan_edited_short', { time: clock(edited) })}</span>{/if}
          {#if pending && failure === null}<span class="sending-mark" title={$t('chan_sending')} aria-label={$t('chan_sending')}><Icon name="clock" size={11} /></span>{/if}
        </span>
      </div>
      {@render more(false)}
    </header>

    {@render marks()}

    {#if item.deleted}
      <p class="tomb"><Icon name="trash-2" size={12} />{$t('chan_post_deleted')}</p>
    {:else}
      {#each words as w (w.id)}<div class="text"><MessageContent text={w.text ?? ''} rich={w.rich ?? null} /></div>{/each}
      {#if album}
        <div class="album">
          {#if rows.length}
            <div class="mosaic">
              {#each rows as row, r (r)}
                <div class="mosaic-row" style:height={rowHeight(row.length, rows.length) ? `${rowHeight(row.length, rows.length)}px` : undefined}>
                  {#each row as p (p.id)}<div class="cell"><ChannelMedia item={p} tile={rows.length > 1 || row.length > 1} /></div>{/each}
                </div>
              {/each}
            </div>
          {/if}
          {#each album.files as p (p.id)}<div class="media"><ChannelMedia item={p} /></div>{/each}
        </div>
      {:else if item.media_json}<div class="media"><ChannelMedia {item} /></div>{/if}
    {/if}

    {#if !item.deleted && !local && (item.reactions.length || onreply)}
      <footer class="foot">
        {@render reactions()}
        <span class="spacer"></span>
        {#if onreply}
          <button class="link reply" onclick={() => onreply?.(item)}><Icon name="reply" size={14} />{$t('chan_reply')}</button>
        {/if}
      </footer>
    {/if}
  {:else}
    <!-- The section (in «Все», and on the post page) a chip on a line of its own above the title, the menu at the
         right of that line (the mockups c6.js, c10.js): the title keeps the whole width at any width of the feed.
         The title row: the title, the pin beside it, and the menu when there is no section line; a post without
         words has its menu on its pictures. -->
    {#if title || !(menuOnMedia || menuBeside)}
      {@const titled = !!title || item.deleted || !section}
      {#if section}
        <div class="section-line">
          {@render sectionMark()}
          {#if item.pinned && !titled}<span class="pin" title={$t('chan_pinned')}><Icon name="pin" size={16} /></span>{/if}
          {#if !full}{@render more(false)}{/if}
        </div>
      {/if}
      {#if titled}
        <header class="title-row">
          {#if title}
            <svelte:element this={full ? 'h2' : 'h3'} class="title" bind:this={titleEl}>{title}</svelte:element>
          {:else if item.deleted}<p class="tomb row-tomb"><Icon name="trash-2" size={12} />{$t('chan_post_deleted')}</p>
          {:else}<span class="spacer"></span>{/if}
          {#if item.pinned || (!full && !section)}
            <span class="tags">
              {#if item.pinned}<span class="pin" title={$t('chan_pinned')}><Icon name="pin" size={16} /></span>{/if}
              {#if !full && !section}{@render more(false)}{/if}
            </span>
          {/if}
        </header>
      {/if}
    {/if}

    {@render marks()}

    {#if !item.deleted}
      {#if album}
        {#if rows.length}
          <div class="visual">
            <div class="mosaic">
              {#each rows as row, r (r)}
                <div class="mosaic-row" style:height={rowHeight(row.length, rows.length) ? `${rowHeight(row.length, rows.length)}px` : undefined}>
                  {#each row as p (p.id)}<div class="cell"><ChannelMedia item={p} tile={rows.length > 1 || row.length > 1} picked={pickedOf(p)} /></div>{/each}
                </div>
              {/each}
            </div>
            {#if menuOnMedia && !full}{@render more(true)}{/if}
          </div>
        {/if}
        {#if album.files.length}
          <div class="files">{#each album.files as p (p.id)}<div class="media"><ChannelMedia item={p} picked={pickedOf(p)} /></div>{/each}</div>
        {/if}
      {:else if single}
        <div class="media" class:visual={singleVisual} class:beside={menuBeside} class:rec={!comment && isRecording(mediaOf(single))} aria-label={title ? undefined : mediaName}>
          <ChannelMedia item={single} picked={pickedOf(single)} />
          {#if menuOnMedia && !full}{@render more(true)}{/if}
          {#if menuBeside && !full}<span class="beside-menu">{@render more(false)}</span>{/if}
        </div>
      {/if}

      {#if restParts.length}
        <div class="rest-wrap" style:--cut-top={cut ? `${cut.top}px` : undefined} style:--cut-line={cut ? `${cut.bottom - cut.top}px` : undefined}>
          <div class="rest" class:clamp={!full} class:fade={cut?.edge} style:max-height={full ? undefined : cut ? `${cut.height}px` : measured ? 'none' : undefined}>
            <div class="rest-inner" bind:this={restInner}>
              {#each restParts as w (w.id)}<div class="text"><MessageContent text={w.text} rich={w.rich} /></div>{/each}
            </div>
          </div>
          {#if cut && !full}
            <span class="dots" aria-hidden="true" style:top={`${cut.top}px`} style:height={`${cut.bottom - cut.top}px`}
              style:left={cut.edge ? undefined : `${cut.right + 2}px`} style:right={cut.edge ? '0' : undefined}>…</span>
          {/if}
        </div>
      {/if}
      {#if overflow && !full && onread}
        <button class="read" onclick={() => onread?.(item)}>{$t('chan_read_more')}<Icon name="chevron-right" size={14} /></button>
      {/if}
    {/if}

    {@render reactions()}

    <footer class="byline">
      <Avatar url={nameStore.picture(item.author)} label={name} seed={item.author} size={full ? 28 : 22} />
      <span class="author">{name}</span>
      {#if full && role !== 'member'}<span class="role {role}">{$t(`chan_role_${role}` as 'chan_role_admin')}</span>{/if}
      <span class="stamp">
        {#if !full}<span class="sep" aria-hidden="true">·</span>{/if}<span title={stamp(item.created_at)}>{full ? listStamp(item.created_at) : clock(item.created_at)}</span>
        {#if edited}<span class="sep" aria-hidden="true">·</span><span class="edited" title={$t('chan_edited_full', { when: stamp(edited) })}>{$t('chan_edited_short', { time: clock(edited) })}</span>{/if}
        {#if pending && failure === null}<span class="sending-mark" title={$t('chan_sending')} aria-label={$t('chan_sending')}><Icon name="clock" size={11} /></span>{/if}
      </span>
      <span class="spacer"></span>
      {#if commentable}
        <button class="link comments" class:go={!item.comments} onclick={() => oncomments?.(item)}
          aria-label={item.comments ? $t('chan_comments_n', { n: String(item.comments) }) : $t('chan_comments_none')}>
          {#if item.comments}<Icon name="message-circle" size={16} />{item.comments}{:else}{$t('chan_comments_none')}{/if}
          <Icon name="chevron-right" size={14} />
        </button>
      {/if}
    </footer>
  {/if}
</article>

<style>
  .post {
    position: relative; display: flex; flex-direction: column; gap: var(--sp-2); width: min(680px, 100%); align-self: center;
    transition: box-shadow var(--dur-base) var(--ease);
  }
  .post.card, .post.comment { border-radius: var(--radius-card, 18px); background: var(--bubble-in-bg); border: 1px solid var(--bubble-in-border); }
  /* A card of the feed (the redesign of the feed): 20 round, the title row, the words, the reactions, who at its foot. */
  .post.card { width: min(720px, 100%); padding: 14px var(--sp-4) var(--sp-3) var(--sp-5); gap: 10px; border-radius: var(--radius-pane); box-sizing: border-box; }
  .post.phone.card { width: min(680px, 100%); padding: var(--sp-3) var(--sp-3) 10px var(--sp-4); }
  .post.full { width: 100%; gap: var(--sp-3); }
  /* A comment: a card of its own (the mockup c4.js), 16 round. */
  /*
   * A comment of a thread (the mockup c6.js): no box, the avatar 32 in a column of its own, beside it one line of
   * the name, the role, the time and ⋮, then its words, its file, the reactions and «Ответить».
   */
  .post.comment {
    width: 100%; display: grid; grid-template-columns: 32px minmax(0, 1fr) auto; align-items: start; column-gap: var(--sp-3); row-gap: var(--sp-1);
    padding: var(--sp-2) 0; border: none; border-radius: 0; background: none;
  }
  .comment .head { display: contents; }
  .comment > :not(.head) { grid-column: 2 / -1; min-width: 0; }
  .comment > .mark { justify-self: start; }
  /* Mine that waits or was given up keeps an edge, so it reads apart. */
  .post.comment.pending, .post.comment.unsent { padding: var(--sp-2) var(--sp-3); border: 1px dashed var(--warn-border); border-radius: var(--radius-lg); }
  .post.comment.unsent { border-style: solid; border-color: var(--danger-border); }
  .post.highlighted { box-shadow: 0 0 0 2px var(--accent-border); }
  .post.pending.card { border-style: dashed; border-color: var(--warn-border); }
  .post.hidden { opacity: 0.7; }
  /* Mine on its way: dimmed with a clock until the runtime tells of it; given up: an edge of danger. */
  .post.sending { opacity: 0.72; }
  .post.unsent { border-color: var(--danger-border); }
  .sending-mark { display: inline-flex; margin-left: 4px; color: var(--text-3); }
  .unsent-mark { gap: 6px; }
  .retry {
    display: inline-flex; align-items: center; gap: 3px; margin-left: 2px; padding: 0 2px; border: none; background: none; cursor: pointer;
    font: inherit; font-weight: var(--fw-bold); color: inherit; text-decoration: underline; text-underline-offset: 2px;
  }

  /* A comment's head: who and when on top. */
  .head { display: flex; align-items: flex-start; gap: var(--sp-2); min-width: 0; }
  .who { flex: 1; min-width: 0; display: flex; flex-wrap: wrap; align-items: center; column-gap: var(--sp-2); row-gap: 2px; min-height: 32px; }
  .name { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .name-text { font-size: var(--fs-base); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .when { display: inline-flex; align-items: center; gap: 3px; font-size: var(--fs-xs); color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  /* A moderator's or an author's chip is drawn by its edge: a grey fill on the post's own fill is lost in the dark. */
  .role { flex-shrink: 0; display: inline-flex; align-items: center; height: 20px; box-sizing: border-box; font-size: var(--fs-2xs); font-weight: var(--fw-bold); padding: 0 var(--sp-2); border-radius: var(--radius-pill); border: 1px solid var(--border-strong); background: transparent; color: var(--text-2); }
  .role.owner { border-color: transparent; background: var(--accent-tint); color: var(--accent-text-2); }
  .role.admin { border-color: transparent; background: var(--success-bg); color: var(--success-text, var(--success)); }
  .sep { margin-inline: 3px; }

  /* The line of the section above the title: its chip, the menu at its right (the mockups c6.js, c10.js). */
  .section-line { display: flex; align-items: center; gap: 6px; min-width: 0; min-height: 32px; margin-right: -6px; }
  .section-line .more { width: 32px; height: 32px; padding: 0; margin-left: auto; color: var(--text-3); }
  .section-line .more:hover { color: var(--text); }
  .full .section-line { min-height: 0; margin-right: 0; }
  /* A post's title row: the title the whole width, the pin beside it (and the menu, when no section line holds it). */
  .title-row { display: flex; align-items: flex-start; gap: var(--sp-3); min-width: 0; margin-right: -6px; }
  .section-line + .title-row { margin-top: -2px; }
  .tags { flex: none; display: flex; align-items: center; gap: 6px; min-height: 32px; min-width: 0; margin-left: auto; }
  .title {
    flex: 1; min-width: 0; margin: 0; padding-top: var(--sp-1); font-size: var(--fs-md); line-height: 23px; font-weight: var(--fw-bold); color: var(--text);
    overflow-wrap: anywhere; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 3; line-clamp: 3; overflow: hidden;
  }
  /* The post page: the title 20, the whole width; the pin beside it, on its first line. */
  .full .title { font-size: var(--fs-xl); line-height: 26px; letter-spacing: -0.01em; display: block; overflow: visible; user-select: text; padding-top: 0; }
  .full .title-row { margin-right: 0; }
  .full .tags { min-height: 26px; }
  .pin { display: inline-flex; flex-shrink: 0; color: var(--accent-text); }
  .scope-mark {
    display: inline-flex; align-items: center; gap: 5px; min-width: 0; flex-shrink: 1; height: 24px; padding: 0 var(--sp-2); box-sizing: border-box;
    border-radius: var(--radius-pill); font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--accent-text-2); background: var(--accent-tint);
  }
  /* A section of its own colour (stage 4): its words in it, its fill a tint of it. */
  .scope-mark.tinted { color: var(--tag); background: color-mix(in srgb, var(--tag) 14%, transparent); }
  .scope-mark :global(svg) { flex-shrink: 0; }
  .scope-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .more { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; align-items: center; justify-content: center; padding: 3px; border-radius: var(--radius); flex-shrink: 0; }
  .tags .more, .beside-menu .more { width: 32px; height: 32px; padding: 0; }
  /* The menu of a comment as the actions beside a message (mockups c1.js): round 32, muted, a fill under the pointer only. */
  .comment .more { width: 32px; height: 32px; padding: 0; margin-right: -6px; border-radius: 50%; color: var(--text-2); }
  .comment .more:hover { color: var(--text); background: var(--surface-hover); }
  .comment .more:focus-visible, .reply:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .more:hover { color: var(--text); background: var(--surface-3); }
  /* The menu of a post without words, on the corner of its pictures. */
  .more.over { position: absolute; top: 8px; right: 8px; z-index: 1; padding: 4px; border-radius: 50%; background: rgba(0, 0, 0, 0.45); color: #fff; }
  .more.over:hover { background: rgba(0, 0, 0, 0.6); color: #fff; }

  .mark { display: inline-flex; align-items: center; gap: 5px; align-self: flex-start; height: 24px; box-sizing: border-box; font-size: var(--fs-xs); font-weight: var(--fw-semibold); padding: 0 var(--sp-2); border-radius: var(--radius-pill); background: var(--surface-3); color: var(--text-2); }
  .mark.warn { background: var(--warn-bg); color: var(--warn-text); }
  .mark.danger { background: var(--danger-bg); color: var(--danger-text); }
  .text { color: var(--text); }
  .card .text, .full .text { color: var(--text-body); }
  .comment .text :global(.text) { font-size: var(--fs-base); line-height: 1.45; }
  .media { display: flex; flex-direction: column; gap: 4px; }
  /* Pictures as one block: the menu of a post without words sits on its corner. */
  .visual { position: relative; width: min(420px, 100%); }
  /* A voice message or a circle with no words: its menu at the right of its row. */
  .media.beside { position: relative; padding-right: 32px; }
  .beside-menu { position: absolute; top: 0; right: 0; }
  /* A voice message or a circle of a post stands in the middle of the card (owner, 2026-10-23); the menu beside it keeps the middle true. */
  .media.rec :global(.rec) { justify-content: center; }
  .media.rec.beside { padding-left: 32px; }
  .files { display: flex; flex-direction: column; gap: 6px; }
  /* An album: rows of one to three pictures, 2px apart, in one rounded frame (the messenger's mosaic). */
  .album { display: flex; flex-direction: column; gap: 6px; }
  .mosaic { display: flex; flex-direction: column; gap: 2px; width: min(420px, 100%); border-radius: var(--radius-md); overflow: hidden; }
  .mosaic-row { display: flex; gap: 2px; min-height: 0; }
  .cell { flex: 1 1 0; min-width: 0; height: 100%; display: flex; }
  .cell :global(.picture) { width: 100%; }
  .row-tomb { flex: 1; padding-top: 4px; }
  .tomb { margin: 0; display: inline-flex; align-items: center; gap: 5px; font-size: var(--fs-xs); color: var(--text-3); font-style: italic; }

  /*
   * The words under the title: three lines in the feed, Markdown blocks
   * included. The box is cut by its height at the bottom of the third line
   * as drawn (cutAt), not by -webkit-line-clamp, which counts the lines of
   * one block; «…» stands after the last word of that line, or, when the
   * line runs to the edge, over its end, which fades out.
   */
  .rest-wrap { position: relative; }
  .rest { font-size: var(--fs-body); line-height: var(--lh-body); }
  /* Until it is measured (and without a layout, in a test) the box holds three lines of plain words. */
  .rest.clamp { max-height: calc(var(--fs-body) * var(--lh-body) * 3); overflow: hidden; }
  .rest-inner { display: flex; flex-direction: column; gap: 6px; }
  .rest.clamp.fade {
    -webkit-mask: linear-gradient(#000, #000) top / 100% var(--cut-top) no-repeat, linear-gradient(to right, #000 calc(100% - 64px), transparent calc(100% - 16px)) 0 var(--cut-top) / 100% var(--cut-line) no-repeat;
    mask: linear-gradient(#000, #000) top / 100% var(--cut-top) no-repeat, linear-gradient(to right, #000 calc(100% - 64px), transparent calc(100% - 16px)) 0 var(--cut-top) / 100% var(--cut-line) no-repeat;
  }
  .dots { position: absolute; display: inline-flex; align-items: center; font-size: var(--fs-body); line-height: 1; color: var(--text-2); pointer-events: none; }
  .read {
    align-self: flex-start; display: inline-flex; align-items: center; gap: 2px; margin-top: -4px; padding: 2px 0; border: none; background: none; cursor: pointer;
    font: inherit; font-size: var(--fs-base); font-weight: var(--fw-semibold); color: var(--accent-text);
  }
  .read:hover { text-decoration: underline; text-underline-offset: 2px; }

  .reactions { display: flex; flex-wrap: wrap; gap: 6px; }
  /* A reaction's pill: its own class, not the global `.chip` (another height, fill and hover). */
  .react {
    display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 10px; border: 1px solid var(--border); border-radius: var(--radius-pill);
    background: var(--surface-3); color: var(--text-2); font: inherit; line-height: 1; cursor: pointer;
  }
  .react:hover { background: var(--surface-hover); }
  .react.mine { border-color: var(--accent-border); background: var(--accent-bg); color: var(--accent-text); }
  .emoji { font-size: 15px; }
  .count { font-family: var(--font-mono); font-size: var(--fs-sm); font-weight: var(--fw-semibold); font-variant-numeric: tabular-nums; }

  /* A comment's foot: its reactions, «Ответить». */
  .foot { display: flex; align-items: center; gap: var(--sp-3); flex-wrap: wrap; padding-top: 2px; }
  /* «Ответить» right after the reactions, not at the far end. */
  .foot .spacer { display: none; }
  .spacer { flex: 1; }
  /* A post's foot: who, when, edited when; its comments on the right. */
  .byline {
    display: flex; align-items: center; gap: 6px; min-width: 0; min-height: 36px; box-sizing: border-box; padding-top: 10px; margin-right: -6px; border-top: 1px solid var(--divider);
    font-size: var(--fs-sm); color: var(--text-3);
  }
  .author { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: var(--fw-semibold); color: var(--text-2); }
  .stamp { display: inline-flex; align-items: center; white-space: nowrap; flex-shrink: 0; }
  .full .stamp { margin-left: 2px; font-size: var(--fs-xs); }
  /* The post page: who wrote it, a line under the words above a thin line (the mockup c6.js). */
  .full .byline { gap: var(--sp-2); padding-top: var(--sp-3); margin-right: 0; }
  .full .author { font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); }
  .edited { color: var(--text-2); text-decoration: underline dotted; text-underline-offset: 3px; cursor: default; }
  .link {
    display: inline-flex; align-items: center; gap: 5px; border: none; background: none; cursor: pointer; padding: 4px 6px; border-radius: var(--radius-sm);
    font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--accent-text-2);
  }
  .link:hover { background: var(--accent-tint); }
  /* «Ответить» under a comment: a quiet link, 13 (the mockup c6.js). */
  .link.reply { gap: var(--sp-1); padding: 4px 0; font-size: var(--fs-sm); color: var(--text-3); }
  .link.reply:hover { background: none; color: var(--text); }
  /* The way to the comments: their count in the quiet text; none yet, «Комментировать» in the accent. */
  .comments { flex-shrink: 0; gap: 5px; height: 32px; padding: 0 6px 0 var(--sp-2); font-weight: var(--fw-semibold); font-size: var(--fs-sm); color: var(--text-2); border-radius: var(--radius); }
  .comments:hover { background: var(--surface-3); }
  .comments.go { color: var(--accent-text); }
  .comments.go:hover { background: var(--accent-tint); }
  .react:focus-visible, .comments:focus-visible, .tags .more:focus-visible, .section-line .more:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  @media (max-width: 599px) { .phone.card { gap: var(--sp-2); } }
  @media (pointer: coarse) {
    .post { -webkit-touch-callout: none; user-select: none; -webkit-user-select: none; }
    .comment .more { width: 40px; height: 40px; padding: 0; margin: -4px -8px -4px 0; }
    .more { padding: 8px; margin: -5px -5px -5px 0; }
    .tags .more, .beside-menu .more, .section-line .more { width: 40px; height: 40px; margin: -4px -4px -4px 0; }
    .section-line .more { margin-left: auto; }
    .link.reply { padding: 8px 0; }
    .more.over { padding: 8px; margin: 0; }
    .link { padding: 8px; }
    .read { padding: 6px 0; }
  }
</style>

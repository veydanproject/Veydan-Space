<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->
<!-- Copied from ui/src/lib/messenger/groups/moderation/QueueTab.svelte at 8ce09c39, then changed for channels. -->

<!--
  What waits for a moderator in a channel: one card per author, with how
  many of theirs wait. A decision on a card takes everything of its author
  («одобрить всё от автора»); the card opens the author's messages, each
  with its own decision (AuthorQueue). Ticked cards are decided together.
  Spam is a ban of the author and the hiding of what they sent.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { Avatar, listStamp, myKey, stamp } from '$lib/messenger/seams/shared';
  import { nameStore } from '../names.svelte';
  import { channelStore } from '../channelStore.svelte';
  import { canBan } from '../permissions';
  import { moderationError } from '../errors';
  import { channelsApi, type ChannelDetail, type QueueItem } from '../api';
  import AuthorQueue from './AuthorQueue.svelte';
  import { actionsFor, confirmDecision, waitingOf, type Decision } from './queue';

  interface Props {
    detail: ChannelDetail;
  }
  let { detail }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const PAGE = 50;
  const cid = $derived(detail.channel.id);
  const me = $derived(myKey());

  let items = $state<QueueItem[]>([]);
  let total = $state(0);
  let next = $state<number | undefined>(undefined);
  let loading = $state(false);
  let busy = $state<string | null>(null);
  let error = $state('');
  /** Ticked cards, by author. */
  let selected = $state<string[]>([]);
  /** The author whose messages are open. */
  let author = $state<string | null>(null);

  async function load(more = false) {
    const id = cid;
    loading = true; error = '';
    try {
      const page = await channelsApi.moderation.queue(id, more ? next : undefined, PAGE);
      if (id !== cid) return;
      items = more ? [...items, ...page.items.filter((x) => !items.some((y) => y.message_id === x.message_id))] : page.items;
      total = page.total;
      next = page.next_before ?? undefined;
      const shown = new Set(items.map((x) => x.author));
      selected = selected.filter((a) => shown.has(a));
    } catch (e) {
      error = moderationError(e, tr);
    } finally {
      loading = false;
    }
  }

  // Again when the channel's count changes (a decision here, on another
  // device of mine, or by another moderator).
  $effect(() => {
    void channelStore.queue[cid];
    if (untrack(() => author)) return;
    untrack(() => load());
  });

  const allTicked = $derived(items.length > 0 && items.every((x) => selected.includes(x.author)));
  const ticked = $derived(items.filter((x) => selected.includes(x.author)));
  /** How many of the card's author wait. */
  const count = (row: QueueItem) => row.more_from_author + 1;

  function tick(a: string) {
    selected = selected.includes(a) ? selected.filter((x) => x !== a) : [...selected, a];
  }

  /**
   * A decision for whole cards: every message of their authors, read anew,
   * so what came after the card was drawn is decided too.
   */
  async function decide(rows: QueueItem[], kind: Decision) {
    if (!rows.length || busy) return;
    const authors = [...new Set(rows.map((r) => r.author))];
    const n = rows.reduce((sum, r) => sum + count(r), 0);
    if (!(await confirmDecision(kind, authors.map((a) => nameStore.label(a)), n))) return;
    busy = rows.length === 1 ? rows[0].author : '*'; error = '';
    try {
      const all = kind === 'spam' ? rows : (await Promise.all(authors.map((a) => waitingOf(cid, a)))).flat();
      for (const a of actionsFor(all, kind)) await channelsApi.moderation.mod(cid, a);
      selected = selected.filter((a) => !authors.includes(a));
    } catch (e) {
      error = moderationError(e, tr);
    } finally {
      busy = null;
    }
    await load();
    channelStore.refreshCounts(cid).catch(() => {});
  }

  function back() {
    author = null;
    load();
  }
</script>

{#if author}
  <AuthorQueue {detail} {author} onback={back} />
{:else}
{#if error}<div class="error-msg">{error}</div>{/if}

{#if items.length}
  <div class="bulk">
    <label class="tick-all">
      <input type="checkbox" checked={allTicked} aria-label={$t('chan_mod_select_all')} onchange={() => (selected = allTicked ? [] : items.map((x) => x.author))} />
      <span>{ticked.length ? $t('chan_mod_selected', { n: String(ticked.length) }) : $t('chan_mod_queue_total', { n: String(total) })}</span>
    </label>
    {#if ticked.length}
      <div class="bulk-acts">
        <!-- One order on every tab: the danger first, the yes last, on the right (as the requests). -->
        {#if ticked.every((r) => canBan(detail, r.author, me))}<button class="btn btn-ghost btn-sm danger" disabled={!!busy} onclick={() => decide(ticked, 'spam')}>{$t('chan_mod_spam')}</button>{/if}
        <button class="btn btn-ghost btn-sm" disabled={!!busy} onclick={() => decide(ticked, 'reject')}>{$t('chan_mod_reject_all')}</button>
        <button class="btn btn-primary btn-sm" disabled={!!busy} onclick={() => decide(ticked, 'approve')}>{$t('chan_mod_approve_all')}</button>
      </div>
    {/if}
  </div>

  <ul class="queue">
    {#each items as row (row.message_id)}
      {@const name = nameStore.label(row.author)}
      {@const more = row.more_from_author}
      {@const n = count(row)}
      <li class="card" class:busy={busy === row.author || busy === '*'}>
        <div class="top">
          <input class="tick" type="checkbox" checked={selected.includes(row.author)} onchange={() => tick(row.author)} aria-label={name} />
          <button class="open" onclick={() => (author = row.author)} aria-label={$t('chan_mod_author_open', { name })} title={$t('chan_mod_author_open', { name })}>
            <Avatar url={nameStore.picture(row.author)} label={name} seed={row.author} size={36} />
            <span class="who">
              <span class="line">
                <span class="name">{name}</span>
                {#if more > 0}<span class="many" title={$t('chan_mod_more_waiting', { n: String(more) })}><Icon name="message-circle" size={12} />{n}</span>{/if}
                <span class="when" title={stamp(row.at)}>{listStamp(row.at)}</span>
              </span>
              <span class="text">{row.text ?? $t('chan_post_file')}</span>
            </span>
            <Icon name="chevron-right" size={16} />
          </button>
        </div>
        <div class="acts">
          {#if canBan(detail, row.author, me)}
            <button class="btn btn-ghost spam" disabled={!!busy} title={$t('chan_mod_spam')} aria-label={$t('chan_mod_spam')} onclick={() => decide([row], 'spam')}><Icon name="ban" size={16} /></button>
          {/if}
          <button class="btn btn-ghost" disabled={!!busy} onclick={() => decide([row], 'reject')}>
            <Icon name="x" size={16} />{n > 1 ? $t('chan_mod_reject_all') : $t('chan_mod_reject')}
          </button>
          <button class="btn btn-primary" disabled={!!busy} onclick={() => decide([row], 'approve')}>
            <Icon name="check" size={16} />{n > 1 ? $t('chan_mod_approve_all') : $t('chan_mod_approve')}
          </button>
        </div>
      </li>
    {/each}
  </ul>
{:else if !loading && !error}
  <div class="empty-state">
    <div class="empty-icon"><Icon name="shield-check" size={24} /></div>
    <p>{$t('chan_mod_queue_empty')}</p>
    <span>{detail.premod === 'off' ? $t('chan_mod_queue_off_hint') : $t('chan_mod_queue_empty_hint')}</span>
  </div>
{/if}

{#if loading}<div class="loading"><Icon name="loader" size={14} /></div>
{:else if next}<button class="btn btn-ghost btn-sm more" onclick={() => load(true)}>{$t('chan_load_more')}</button>{/if}
{/if}

<style>
  .bulk { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); flex-wrap: wrap; padding: 0 4px; }
  .tick-all { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-xs); color: var(--text-2); cursor: pointer; user-select: none; }
  .bulk-acts { display: flex; gap: 6px; flex-wrap: wrap; }
  .queue { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .card { display: flex; flex-direction: column; gap: var(--sp-3); padding: var(--sp-3); border-radius: var(--radius); background: var(--surface-2); border: 1px solid var(--border); container-type: inline-size; }
  .card.busy { opacity: 0.6; }
  .top { display: flex; gap: var(--sp-2); align-items: flex-start; }
  .open {
    flex: 1; min-width: 0; display: flex; gap: var(--sp-2); align-items: flex-start; padding: 0; border: none; background: none;
    color: inherit; font: inherit; text-align: left; cursor: pointer;
  }
  .open > :global(svg) { flex-shrink: 0; margin-top: 10px; color: var(--text-3); }
  .tick { margin-top: 10px; flex-shrink: 0; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .line { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .many { flex-shrink: 0; display: inline-flex; align-items: center; gap: 3px; font-size: var(--fs-2xs); font-weight: var(--fw-bold); padding: 1px 7px; border-radius: var(--radius-pill); background: var(--accent-bg, var(--surface-3)); color: var(--accent-text, var(--text-2)); }
  .when { margin-left: auto; flex-shrink: 0; font-size: var(--fs-2xs); color: var(--text-3); }
  .text { font-size: var(--fs-sm); color: var(--text-body); overflow: hidden; display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow-wrap: anywhere; }
  .acts { display: flex; gap: var(--sp-2); }
  .acts .btn { flex: 1 1 0; min-width: 0; padding-inline: var(--sp-2); }
  .acts .spam { flex: 0 0 auto; padding-inline: var(--sp-3); }
  .danger, .spam { color: var(--danger-text); }
  @container (max-width: 360px) {
    .acts { gap: 6px; }
    .acts .btn { padding-inline: 6px; font-size: var(--fs-sm); }
    .acts .btn:not(.spam) > :global(svg) { display: none; }
  }
  .empty-state { padding: var(--sp-8) var(--sp-4); }
  .empty-state p { margin: 0; }
  .loading { display: flex; justify-content: center; color: var(--text-3); padding: var(--sp-2); }
  .more { align-self: center; }
  @media (pointer: coarse) { .acts .btn { min-height: 44px; } }
</style>

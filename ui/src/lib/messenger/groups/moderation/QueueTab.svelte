<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  What waits for a moderator in a group. One row per author, with how
  many of theirs wait, so a flood of one person is one decision: the
  buttons of a card decide everything of theirs under the filter, a tap
  on it opens every message of theirs to decide one by one. One search
  reads the text, the author's name and their key; there is no list of
  authors to pick from, a group of thousands would make it endless.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../../contacts/Avatar.svelte';
  import AuthorQueue from './AuthorQueue.svelte';
  import { nameStore } from '../names.svelte';
  import { groupStore } from '../groupStore.svelte';
  import { canSpam } from '../permissions';
  import { moderationError } from '../errors';
  import { listStamp, stamp } from '../../shared/time';
  import { messengerApi, type MessengerGroup, type QueueFilter, type QueueItem } from '../../api';
  import { periodSince, QUEUE_PERIODS, type QueuePeriod } from './schedule';
  import { actionsFor, confirmDecision, waitingOf, type Decision } from './queue';

  interface Props {
    group: MessengerGroup;
  }
  let { group }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'msg_you', params);
  const PAGE = 50;

  let period = $state<QueuePeriod>('all');
  let search = $state('');
  /** The search as typed a moment ago: a page is not read for every letter. */
  let text = $state('');
  let items = $state<QueueItem[]>([]);
  let total = $state(0);
  let next = $state<number | undefined>(undefined);
  let loading = $state(false);
  let busy = $state<string | null>(null);
  let error = $state('');
  /** Authors ticked, by key. */
  let selected = $state<string[]>([]);
  /** The author whose messages are open. */
  let author = $state<string | null>(null);

  $effect(() => {
    const typed = search;
    const timer = setTimeout(() => (text = typed.trim()), 300);
    return () => clearTimeout(timer);
  });

  const filter = $derived<QueueFilter>({
    ...(periodSince(period, new Date()) !== undefined ? { since: periodSince(period, new Date()) } : {}),
    ...(text ? { text } : {}),
  });
  const filtered = $derived(period !== 'all' || !!text);

  async function load(more = false) {
    const f = filter;
    loading = true; error = '';
    try {
      const page = await messengerApi.moderation.groupQueue(group.id, f, more ? next : undefined, PAGE);
      if (f !== filter) return;
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

  // Again when the filter changes, and when the group's count does (a
  // decision here, on another device of mine, or by another moderator).
  $effect(() => {
    void filter; void group.queue;
    if (untrack(() => author)) return;
    untrack(() => load());
  });

  const allTicked = $derived(items.length > 0 && items.every((x) => selected.includes(x.author)));
  const ticked = $derived(items.filter((x) => selected.includes(x.author)));
  const count = (row: QueueItem) => row.more_from_author + 1;

  function tick(a: string) {
    selected = selected.includes(a) ? selected.filter((x) => x !== a) : [...selected, a];
  }

  /**
   * A decision for whole cards: every message of their authors under the
   * filter, read anew, so what came after the card was drawn is decided too.
   */
  async function decide(rows: QueueItem[], kind: Decision) {
    if (!rows.length || busy) return;
    const authors = [...new Set(rows.map((r) => r.author))];
    const n = rows.reduce((sum, r) => sum + count(r), 0);
    if (!(await confirmDecision(kind, authors.map((a) => nameStore.label(a)), n))) return;
    busy = rows.length === 1 ? rows[0].author : '*'; error = '';
    try {
      const all = kind === 'spam' ? rows : (await Promise.all(authors.map((a) => waitingOf(group.id, filter, a)))).flat();
      for (const a of actionsFor(all, kind)) await messengerApi.moderation.groupMod(group.id, a);
      selected = selected.filter((a) => !authors.includes(a));
    } catch (e) {
      error = moderationError(e, tr);
    } finally {
      busy = null;
    }
    await load();
    groupStore.refresh(group.id).catch(() => {});
  }

  function back() {
    author = null;
    load();
  }
</script>

{#if author}
  <AuthorQueue {group} {author} {filter} onback={back} />
{:else}
  <div class="search">
    <Icon name="search" size={16} />
    <input type="search" bind:value={search} placeholder={$t('msg_mod_search')} aria-label={$t('msg_mod_search')} />
  </div>

  <!-- One quiet line over the list: «Ждут: N» (all ticked) on the left, the periods on the right. -->
  <div class="listbar">
    {#if items.length}
      <label class="tick-all">
        <input type="checkbox" checked={allTicked} aria-label={$t('msg_mod_select_all')} onchange={() => (selected = allTicked ? [] : items.map((x) => x.author))} />
        <span>{ticked.length ? $t('msg_mod_selected', { n: String(ticked.length) }) : $t('msg_mod_queue_total', { n: String(total) })}</span>
      </label>
    {/if}
    <div class="seg periods" role="radiogroup" aria-label={$t('msg_mod_tab_queue')}>
      {#each QUEUE_PERIODS as p (p)}
        <button class="seg-btn" class:active={period === p} role="radio" aria-checked={period === p} onclick={() => (period = p)}>
          {$t(`msg_mod_period_${p}` as 'msg_mod_period_all')}
        </button>
      {/each}
    </div>
  </div>

  {#if error}<div class="error-msg">{error}</div>{/if}

  {#if items.length}
    {#if ticked.length}
      <div class="bulk-acts">
        <button class="btn btn-primary btn-sm" disabled={!!busy} onclick={() => decide(ticked, 'approve')}>{$t('msg_mod_approve_all')}</button>
        <button class="btn btn-ghost btn-sm" disabled={!!busy} onclick={() => decide(ticked, 'reject')}>{$t('msg_mod_reject_all')}</button>
        {#if ticked.every((r) => canSpam(group, r.author))}<button class="btn btn-ghost btn-sm danger" disabled={!!busy} onclick={() => decide(ticked, 'spam')}>{$t('msg_mod_spam')}</button>{/if}
      </div>
    {/if}

    <ul class="queue">
      {#each items as row (row.message_id)}
        {@const name = nameStore.label(row.author)}
        {@const n = count(row)}
        <li class="q-row" class:busy={busy === row.author || busy === '*'}>
          <div class="top">
            <input class="tick" type="checkbox" checked={selected.includes(row.author)} onchange={() => tick(row.author)} aria-label={name} />
            <button class="open" onclick={() => (author = row.author)} aria-label={$t('msg_mod_author_open', { name })}>
              <Avatar url={nameStore.picture(row.author)} label={name} seed={row.author} size={32} />
              <span class="who">
                <span class="line">
                  <span class="name">{name}</span>
                  {#if n > 1}<span class="many" title={$t('msg_mod_queue_total', { n: String(n) })}><Icon name="message-circle" size={12} />{n}</span>{/if}
                  <span class="when" title={stamp(row.at)}>{listStamp(row.at)}</span>
                </span>
                <span class="text">{row.text}</span>
              </span>
              <Icon name="chevron-right" size={16} />
            </button>
          </div>
          <div class="acts">
            <button class="btn btn-primary btn-sm" disabled={!!busy} onclick={() => decide([row], 'approve')}>
              {n > 1 ? $t('msg_mod_approve_all') : $t('msg_mod_approve')}
            </button>
            <button class="btn btn-ghost btn-sm" disabled={!!busy} onclick={() => decide([row], 'reject')}>
              {n > 1 ? $t('msg_mod_reject_all') : $t('msg_mod_reject')}
            </button>
            {#if canSpam(group, row.author)}
              <button class="icon-btn spam" disabled={!!busy} title={$t('msg_mod_spam')} aria-label={$t('msg_mod_spam')} onclick={() => decide([row], 'spam')}><Icon name="ban" size={16} /></button>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {:else if !loading && !error}
    <p class="empty">{filtered ? $t('msg_mod_queue_empty_filtered') : $t('msg_mod_queue_empty')}</p>
  {/if}

  {#if loading}<div class="loading"><Icon name="loader" size={14} /></div>
  {:else if next}<button class="btn btn-ghost btn-sm more" onclick={() => load(true)}>{$t('msg_mod_more')}</button>{/if}
{/if}

<style>
  /* The search the whole width, 36 high (c11). */
  .search {
    display: flex; align-items: center; gap: var(--sp-2); min-height: calc(var(--control-h) - var(--sp-1)); padding: 0 var(--sp-3); color: var(--text-3);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius-field);
  }
  .search:focus-within { border-color: var(--accent-border); }
  .search :global(svg) { flex-shrink: 0; }
  .search input { flex: 1; min-width: 0; border: none; background: none; font: inherit; font-size: var(--fs-sm); color: var(--text); padding: 0; align-self: stretch; outline: none; box-shadow: none; }
  .listbar { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; padding-left: var(--sp-1); }
  .tick-all { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-3); cursor: pointer; user-select: none; }
  .periods { margin-left: auto; gap: 2px; padding: 2px; }
  .periods .seg-btn { height: var(--control-h-sm); padding: 0 var(--sp-2); font-size: var(--fs-xs); }
  .seg-btn:focus-visible { outline: 2px solid var(--accent-border); outline-offset: -2px; }
  .bulk-acts { display: flex; gap: 6px; flex-wrap: wrap; }
  /* The queue: one card, a row per author under a divider. */
  .queue { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-lg); overflow: hidden; container-type: inline-size; }
  .q-row { display: grid; grid-template-columns: auto minmax(0, 1fr); column-gap: var(--sp-2); row-gap: var(--sp-2); padding: var(--sp-3); }
  .q-row + .q-row { border-top: 1px solid var(--border); }
  /* The tick and the author share the grid with the answers, so these start under the words. */
  .top { display: contents; }
  .q-row.busy { opacity: 0.6; }
  .tick { margin-top: var(--sp-2); flex-shrink: 0; }
  .open { min-width: 0; display: flex; align-items: flex-start; gap: var(--sp-2); border: none; background: none; padding: 0; color: inherit; font: inherit; cursor: pointer; text-align: left; border-radius: var(--radius-sm); }
  .open:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .open > :global(svg:last-child) { flex-shrink: 0; margin-top: 2px; color: var(--text-3); }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .line { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .name { font-size: var(--fs-base); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .many { flex-shrink: 0; display: inline-flex; align-items: center; gap: 3px; font-size: var(--fs-2xs); font-weight: var(--fw-bold); padding: 1px var(--sp-2); border-radius: var(--radius-pill); background: var(--accent-bg, var(--surface-3)); color: var(--accent-text, var(--text-2)); }
  .when { margin-left: auto; flex-shrink: 0; font-size: var(--fs-xs); color: var(--text-3); }
  .text { font-size: var(--fs-sm); line-height: 1.4; color: var(--text); overflow: hidden; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow-wrap: anywhere; }
  /* Under the words (past the avatar): «Одобрить | Отклонить» halves 30 high, the spam an icon in a third column. */
  .acts { grid-column: 2; display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) calc(var(--control-h-sm) - 2px); align-items: center; gap: var(--sp-2); padding-left: var(--sp-10); }
  .acts .btn { min-width: 0; min-height: calc(var(--control-h-sm) - 2px); padding-inline: var(--sp-3); font-size: var(--fs-xs); line-height: 1.25; white-space: normal; text-wrap: balance; text-align: center; }
  .acts .spam { width: calc(var(--control-h-sm) - 2px); height: calc(var(--control-h-sm) - 2px); }
  .danger, .spam { color: var(--danger-text); }
  /* A narrow panel (beside a chat on a small screen): the answers take the width under the avatar too, so «Одобрить все» fits. */
  @container (max-width: 304px) {
    .acts { padding-left: 0; }
  }
  .empty { margin: 0; font-size: var(--fs-sm); color: var(--text-3); text-align: center; padding: var(--sp-4) 0; }
  .loading { display: flex; justify-content: center; color: var(--text-3); padding: var(--sp-2); }
  .more { align-self: center; }
  @media (pointer: coarse) {
    .search input { font-size: var(--fs-md); min-height: 40px; }
    .acts { grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) var(--control-h); }
    .acts .btn { min-height: var(--control-h); }
    .acts .spam { width: var(--control-h); height: var(--control-h); }
  }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  What waits for a moderator in a group. One row per author (the oldest of
  theirs, and how many more), so a flood of one person is one decision;
  the rest of theirs comes up when that one is decided. A row opens on a
  tap with what was said before it, a decision is taken for a row or for
  the rows ticked.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
  import Avatar from '../../contacts/Avatar.svelte';
  import MessageContent from '../../content/MessageContent.svelte';
  import { nameStore } from '../names.svelte';
  import { groupStore } from '../groupStore.svelte';
  import { canSpam } from '../permissions';
  import { moderationError } from '../errors';
  import { chatStore } from '../../chats/chatStore.svelte';
  import { confirmStore } from '../../shared/confirm.svelte';
  import { clock, stamp } from '../../shared/time';
  import { messengerApi, type MessengerGroup, type MessengerMessage, type ModAction, type QueueFilter, type QueueItem } from '../../api';
  import { periodSince, QUEUE_PERIODS, type QueuePeriod } from './schedule';

  interface Props {
    group: MessengerGroup;
  }
  let { group }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'msg_you', params);
  const PAGE = 50;

  let author = $state<string | null>(null);
  let period = $state<QueuePeriod>('all');
  let search = $state('');
  /** The search as typed a moment ago: a page is not read for every letter. */
  let text = $state('');
  let items = $state<QueueItem[]>([]);
  let total = $state(0);
  let next = $state<number | undefined>(undefined);
  let loading = $state(false);
  let busy = $state(false);
  let error = $state('');
  let selected = $state<string[]>([]);
  let open = $state<string | null>(null);
  /** What was said before an opened row, by message id. */
  let context = $state<Record<string, MessengerMessage[]>>({});

  $effect(() => {
    const typed = search;
    const timer = setTimeout(() => (text = typed.trim()), 300);
    return () => clearTimeout(timer);
  });

  const filter = $derived<QueueFilter>({
    ...(author ? { author } : {}),
    ...(periodSince(period, new Date()) !== undefined ? { since: periodSince(period, new Date()) } : {}),
    ...(text ? { text } : {}),
  });
  const filtered = $derived(!!author || period !== 'all' || !!text);

  async function load(more = false) {
    const f = filter;
    loading = true; error = '';
    try {
      const page = await messengerApi.moderation.groupQueue(group.id, f, more ? next : undefined, PAGE);
      if (f !== filter) return;
      items = more ? [...items, ...page.items.filter((x) => !items.some((y) => y.message_id === x.message_id))] : page.items;
      total = page.total;
      next = page.next_before ?? undefined;
      const shown = new Set(items.map((x) => x.message_id));
      selected = selected.filter((id) => shown.has(id));
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
    untrack(() => load());
  });

  const authors = $derived([
    { label: $t('msg_mod_filter_author_all'), value: null },
    ...group.members.filter((m) => !m.is_me).map((m) => ({ label: nameStore.label(m.pubkey), value: m.pubkey })),
  ]);
  const allTicked = $derived(items.length > 0 && items.every((x) => selected.includes(x.message_id)));
  const ticked = $derived(items.filter((x) => selected.includes(x.message_id)));
  /** Spam bans: offered when I may ban every author ticked. */
  const spamOk = (rows: QueueItem[]) => rows.length > 0 && rows.every((r) => canSpam(group, r.author));

  function tick(id: string) {
    selected = selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id];
  }

  async function decide(rows: QueueItem[], kind: 'approve' | 'reject' | 'spam') {
    if (!rows.length || busy) return;
    const authorsOf = [...new Set(rows.map((r) => r.author))];
    if (kind === 'spam') {
      const names = authorsOf.map((a) => nameStore.label(a)).join(', ');
      if (!(await confirmStore.ask($t('msg_mod_spam_confirm', { names }), $t('msg_mod_spam'), true))) return;
    }
    const actions: ModAction[] = kind === 'approve'
      ? rows.map((r) => ({ t: 'approve', target: r.message_id, version: r.version }))
      : kind === 'reject'
        ? rows.map((r) => ({ t: 'reject', target: r.message_id }))
        : authorsOf.map((a) => ({ t: 'spam', author: a, target: rows.find((r) => r.author === a)?.message_id }));
    busy = true; error = '';
    try {
      for (const a of actions) await messengerApi.moderation.groupMod(group.id, a);
      selected = selected.filter((id) => !rows.some((r) => r.message_id === id));
    } catch (e) {
      error = moderationError(e, tr);
    } finally {
      busy = false;
    }
    await load();
    groupStore.refresh(group.id).catch(() => {});
  }

  /** The two messages before it in the chat: from the open chat when it holds them, else read. */
  async function expand(row: QueueItem) {
    open = open === row.message_id ? null : row.message_id;
    if (!open || context[row.message_id]) return;
    const chatId = `group:${group.id}`;
    const shown = chatStore.activeId === chatId ? chatStore.messages : [];
    const at = shown.findIndex((m) => m.id === row.message_id);
    let before = at > 0 ? shown.slice(Math.max(0, at - 2), at) : [];
    if (at < 0) {
      try { before = (await messengerApi.chats.messages(chatId, row.at, 2)).slice(-2); }
      catch { before = []; }
    }
    context = { ...context, [row.message_id]: before.filter((m) => m.content_type !== 'system') };
  }
</script>

<div class="filters">
  <div class="search">
    <Icon name="search" size={13} />
    <input type="search" bind:value={search} placeholder={$t('msg_mod_search')} aria-label={$t('msg_mod_search')} />
  </div>
  <div class="row">
    <div class="who"><CustomSelect options={authors} bind:value={author} /></div>
    <div class="seg" role="radiogroup" aria-label={$t('msg_mod_tab_queue')}>
      {#each QUEUE_PERIODS as p (p)}
        <button class="seg-btn" class:active={period === p} role="radio" aria-checked={period === p} onclick={() => (period = p)}>
          {$t(`msg_mod_period_${p}` as 'msg_mod_period_all')}
        </button>
      {/each}
    </div>
  </div>
</div>

{#if error}<div class="error-msg">{error}</div>{/if}

{#if items.length}
  <div class="bulk">
    <label class="tick-all">
      <input type="checkbox" checked={allTicked} aria-label={$t('msg_mod_select_all')} onchange={() => (selected = allTicked ? [] : items.map((x) => x.message_id))} />
      <span>{selected.length ? $t('msg_mod_selected', { n: String(selected.length) }) : $t('msg_mod_queue_total', { n: String(total) })}</span>
    </label>
    {#if ticked.length}
      <div class="acts">
        <button class="btn btn-primary btn-sm" disabled={busy} onclick={() => decide(ticked, 'approve')}>{$t('msg_mod_approve')}</button>
        <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => decide(ticked, 'reject')}>{$t('msg_mod_reject')}</button>
        {#if spamOk(ticked)}<button class="btn btn-ghost btn-sm danger" disabled={busy} onclick={() => decide(ticked, 'spam')}>{$t('msg_mod_spam')}</button>{/if}
      </div>
    {/if}
  </div>

  <ul class="queue">
    {#each items as row (row.message_id)}
      {@const name = nameStore.label(row.author)}
      <li class="item" class:open={open === row.message_id}>
        <input class="tick" type="checkbox" checked={selected.includes(row.message_id)} onchange={() => tick(row.message_id)} aria-label={name} />
        <div class="body">
          <button class="head" onclick={() => expand(row)} aria-expanded={open === row.message_id}>
            <Avatar url={nameStore.picture(row.author)} label={name} seed={row.author} size={28} />
            <span class="name">{name}</span>
            {#if row.more_from_author > 0}<span class="more-from">{$t('msg_mod_queue_more_from', { n: String(row.more_from_author) })}</span>{/if}
            <span class="when">{stamp(row.at)}</span>
          </button>
          {#if open === row.message_id}
            {#if context[row.message_id]?.length}
              <div class="context" aria-label={$t('msg_mod_context')}>
                {#each context[row.message_id] as c (c.id)}
                  <div class="said"><b>{nameStore.label(c.sender_pubkey)}</b> <span class="said-at">{clock(c.created_at)}</span> {c.text ?? ''}</div>
                {/each}
              </div>
            {/if}
            <div class="text full"><MessageContent text={row.text} cards={false} /></div>
          {:else}
            <div class="text">{row.text}</div>
          {/if}
          <div class="row-acts">
            <button class="icon ok" disabled={busy} title={$t('msg_mod_approve')} aria-label={$t('msg_mod_approve')} onclick={() => decide([row], 'approve')}><Icon name="check" size={15} /></button>
            <button class="icon" disabled={busy} title={$t('msg_mod_reject')} aria-label={$t('msg_mod_reject')} onclick={() => decide([row], 'reject')}><Icon name="x" size={15} /></button>
            {#if canSpam(group, row.author)}
              <button class="icon danger" disabled={busy} title={$t('msg_mod_spam')} aria-label={$t('msg_mod_spam')} onclick={() => decide([row], 'spam')}><Icon name="ban" size={15} /></button>
            {/if}
          </div>
        </div>
      </li>
    {/each}
  </ul>
{:else if !loading && !error}
  <p class="empty">{filtered ? $t('msg_mod_queue_empty_filtered') : $t('msg_mod_queue_empty')}</p>
{/if}

{#if loading}<div class="loading"><Icon name="loader" size={14} /></div>
{:else if next}<button class="btn btn-ghost btn-sm more" onclick={() => load(true)}>{$t('msg_mod_more')}</button>{/if}

<style>
  .filters { display: flex; flex-direction: column; gap: var(--sp-2); }
  .search {
    display: flex; align-items: center; gap: 6px; padding: 0 10px; color: var(--text-3);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-field);
  }
  .search:focus-within { border-color: var(--accent-border); }
  .search input { flex: 1; min-width: 0; border: none; background: none; font: inherit; font-size: var(--fs-sm); color: var(--text); padding: 7px 0; outline: none; }
  .row { display: flex; gap: var(--sp-2); flex-wrap: wrap; align-items: center; }
  .who { flex: 1 1 140px; min-width: 0; }
  .seg { max-width: 100%; }
  .seg-btn { min-height: 30px; padding-block: 4px; white-space: nowrap; }
  .bulk { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); flex-wrap: wrap; padding: 4px; }
  .tick-all { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-xs); color: var(--text-2); cursor: pointer; user-select: none; }
  .acts { display: flex; gap: 6px; flex-wrap: wrap; }
  .queue { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .item { display: flex; gap: var(--sp-2); padding: 8px 4px; border-radius: var(--radius-sm); align-items: flex-start; }
  .item:hover, .item.open { background: var(--surface-row-hover); }
  .tick { margin-top: 7px; flex-shrink: 0; }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .head { display: flex; align-items: center; gap: var(--sp-2); width: 100%; border: none; background: none; padding: 0; color: inherit; font: inherit; cursor: pointer; text-align: left; min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .more-from { flex-shrink: 0; font-size: var(--fs-2xs); font-weight: var(--fw-bold); padding: 1px 6px; border-radius: var(--radius-pill); background: var(--surface-3); color: var(--text-2); }
  .when { margin-left: auto; flex-shrink: 0; font-size: var(--fs-2xs); color: var(--text-3); }
  .text { font-size: var(--fs-sm); color: var(--text-body); overflow: hidden; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow-wrap: anywhere; }
  .text.full { display: block; }
  .context { display: flex; flex-direction: column; gap: 2px; padding: 4px 8px; border-left: 2px solid var(--border-2); }
  .said { font-size: var(--fs-xs); color: var(--text-3); overflow-wrap: anywhere; }
  .said-at { font-size: var(--fs-2xs); }
  .row-acts { display: flex; gap: 2px; }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .icon:hover:not(:disabled) { color: var(--text); background: var(--surface-3); }
  .icon.ok { color: var(--success-text, var(--success)); }
  .danger { color: var(--danger-text); }
  .empty { margin: 0; font-size: var(--fs-sm); color: var(--text-3); text-align: center; padding: var(--sp-4) 0; }
  .loading { display: flex; justify-content: center; color: var(--text-3); padding: var(--sp-2); }
  .more { align-self: center; }
  @media (pointer: coarse) {
    .icon { padding: 10px; }
    .search input { font-size: 16px; min-height: 40px; }
    .tick { margin-top: 4px; }
  }
</style>

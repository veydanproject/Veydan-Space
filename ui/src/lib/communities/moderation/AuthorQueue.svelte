<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->
<!-- Copied from ui/src/lib/messenger/groups/moderation/AuthorQueue.svelte at 6a06625c, then changed for channels. -->

<!--
  Everything one author has waiting in a channel, opened from their card:
  every post and comment with its own decision, the ticked ones or all of
  them at once (`channel_moderation_queue` with `author`). The queue of a
  channel has no filter of time and its messages no chat around them, so
  there is no context to unfold. When nothing of theirs waits any more,
  the way leads back.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { Avatar, MessageContent, listStamp, myKey, stamp } from '$lib/messenger/seams/shared';
  import { nameStore } from '../names.svelte';
  import { channelStore } from '../channelStore.svelte';
  import { canBan } from '../permissions';
  import { moderationError } from '../errors';
  import { channelsApi, type ChannelDetail, type QueueItem } from '../api';
  import { actionsFor, confirmDecision, waitingOf, type Decision } from './queue';

  interface Props {
    detail: ChannelDetail;
    author: string;
    onback: () => void;
  }
  let { detail, author, onback }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  /** Rows drawn at a time: a flood of hundreds is not one long page. */
  const STEP = 100;
  const cid = $derived(detail.channel.id);

  let items = $state<QueueItem[]>([]);
  let loaded = $state(false);
  let shown = $state(STEP);
  let busy = $state(false);
  let error = $state('');
  let selected = $state<string[]>([]);

  const name = $derived(nameStore.label(author));
  const spamOk = $derived(canBan(detail, author, myKey()));
  const allTicked = $derived(items.length > 0 && items.every((x) => selected.includes(x.message_id)));
  const ticked = $derived(items.filter((x) => selected.includes(x.message_id)));

  async function load() {
    error = '';
    try {
      const rows = await waitingOf(cid, author);
      items = rows;
      loaded = true;
      const ids = new Set(rows.map((x) => x.message_id));
      selected = selected.filter((id) => ids.has(id));
      if (!rows.length) onback();
    } catch (e) {
      error = moderationError(e, tr);
    }
  }

  // Again when the channel's count changes (a decision here, on another device, by another moderator).
  $effect(() => {
    void channelStore.queue[cid];
    untrack(() => load());
  });

  function tick(id: string) {
    selected = selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id];
  }

  async function decide(rows: QueueItem[], kind: Decision) {
    if (!rows.length || busy) return;
    if (!(await confirmDecision(kind, [name], rows.length))) return;
    busy = true; error = '';
    try {
      for (const a of actionsFor(rows, kind)) await channelsApi.moderation.mod(cid, a);
      selected = selected.filter((id) => !rows.some((r) => r.message_id === id));
    } catch (e) {
      error = moderationError(e, tr);
    } finally {
      busy = false;
    }
    await load();
    channelStore.refreshCounts(cid).catch(() => {});
  }
</script>

<!-- The panel's head has the one back arrow (to the channel): the way to the whole queue is a word. -->
<div class="who">
  <Avatar url={nameStore.picture(author)} label={name} seed={author} size={36} />
  <div class="who-text">
    <span class="name">{name}</span>
    {#if loaded}<span class="count">{$t('chan_mod_queue_total', { n: String(items.length) })}</span>{/if}
  </div>
  <button class="btn btn-ghost btn-sm" onclick={onback}><Icon name="list" size={13} />{$t('chan_mod_queue_all')}</button>
</div>

{#if error}<div class="error-msg">{error}</div>{/if}

{#if items.length}
  <div class="bulk">
    <label class="tick-all">
      <input type="checkbox" checked={allTicked} aria-label={$t('chan_mod_select_all')} onchange={() => (selected = allTicked ? [] : items.map((x) => x.message_id))} />
      <span>{ticked.length ? $t('chan_mod_selected', { n: String(ticked.length) }) : $t('chan_mod_select_all')}</span>
    </label>
  </div>
  <!-- The buttons know their width: the panel beside a feed is narrow. -->
  <div class="acts-box"><div class="acts">
    <!-- The order of every tab: the danger first, the yes last, on the right. -->
    {#if spamOk}
      <button class="btn btn-ghost spam" disabled={busy} title={$t('chan_mod_spam')} aria-label={$t('chan_mod_spam')} onclick={() => decide(items, 'spam')}><Icon name="ban" size={16} /></button>
    {/if}
    {#if ticked.length}
      <button class="btn btn-ghost" disabled={busy} onclick={() => decide(ticked, 'reject')}><Icon name="x" size={16} />{$t('chan_mod_reject')}</button>
      <button class="btn btn-primary" disabled={busy} onclick={() => decide(ticked, 'approve')}><Icon name="check" size={16} />{$t('chan_mod_approve')}</button>
    {:else}
      <button class="btn btn-ghost" disabled={busy} onclick={() => decide(items, 'reject')}><Icon name="x" size={16} />{$t('chan_mod_reject_all')}</button>
      <button class="btn btn-primary" disabled={busy} onclick={() => decide(items, 'approve')}><Icon name="check" size={16} />{$t('chan_mod_approve_all')}</button>
    {/if}
  </div></div>

  <ul class="list divided">
    {#each items.slice(0, shown) as row (row.message_id)}
      <li class="msg">
        <input class="tick" type="checkbox" checked={selected.includes(row.message_id)} onchange={() => tick(row.message_id)} aria-label={stamp(row.at)} />
        <div class="body">
          <span class="when" title={stamp(row.at)}>{listStamp(row.at)}</span>
          <div class="text">{#if row.text}<MessageContent text={row.text} cards={false} />{:else}{$t('chan_post_file')}{/if}</div>
        </div>
        <div class="row-acts">
          <button class="icon ok" disabled={busy} title={$t('chan_mod_approve')} aria-label={$t('chan_mod_approve')} onclick={() => decide([row], 'approve')}><Icon name="check" size={18} /></button>
          <button class="icon" disabled={busy} title={$t('chan_mod_reject')} aria-label={$t('chan_mod_reject')} onclick={() => decide([row], 'reject')}><Icon name="x" size={18} /></button>
        </div>
      </li>
    {/each}
  </ul>
  {#if items.length > shown}<button class="btn btn-ghost btn-sm more" onclick={() => (shown += STEP)}>{$t('chan_load_more')}</button>{/if}
{:else if !loaded && !error}
  <div class="loading"><Icon name="loader" size={14} /></div>
{/if}

<style>
  .who { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  .who-text { display: flex; flex-direction: column; min-width: 0; }
  .name { font-size: var(--fs-md); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count { font-size: var(--fs-xs); color: var(--text-3); }
  .bulk { padding: 0 4px; }
  .tick-all { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-xs); color: var(--text-2); cursor: pointer; user-select: none; }
  .acts { display: flex; gap: var(--sp-2); }
  .acts .btn { flex: 1 1 0; min-width: 0; justify-content: center; gap: 6px; min-height: 40px; }
  .acts-box { container-type: inline-size; }
  .acts .spam { flex: 0 0 auto; color: var(--danger-text); padding-inline: var(--sp-3); }
  @container (max-width: 360px) {
    .acts { gap: 6px; }
    .acts .btn { padding-inline: var(--sp-3); font-size: var(--fs-sm); white-space: normal; text-wrap: balance; line-height: 1.25; }
    .acts .btn:not(.spam) > :global(svg) { display: none; }
  }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .msg { display: flex; gap: var(--sp-2); padding: 10px 4px; align-items: flex-start; }
  .tick { margin-top: 3px; flex-shrink: 0; }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .when { font-size: var(--fs-2xs); color: var(--text-3); }
  .text { font-size: var(--fs-sm); color: var(--text-body); overflow-wrap: anywhere; }
  .row-acts { display: flex; gap: 2px; flex-shrink: 0; }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 8px; border-radius: var(--radius-sm); }
  .icon:hover:not(:disabled) { color: var(--text); background: var(--surface-3); }
  .icon.ok { color: var(--success-text, var(--success)); }
  .loading { display: flex; justify-content: center; color: var(--text-3); padding: var(--sp-2); }
  .more { align-self: center; }
  @media (pointer: coarse) {
    .icon { padding: 11px; }
    .acts .btn { min-height: 44px; }
  }
</style>

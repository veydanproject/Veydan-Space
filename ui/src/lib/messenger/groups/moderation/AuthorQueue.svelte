<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Everything one author has waiting under the filter of the queue, opened
  from their card: every message with its own decision, the ticked ones
  or all of them at once. A tap on a message shows what was said before
  it. When nothing of theirs waits any more, the way leads back.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../../contacts/Avatar.svelte';
  import MessageContent from '../../content/MessageContent.svelte';
  import { nameStore } from '../names.svelte';
  import { groupStore } from '../groupStore.svelte';
  import { canSpam } from '../permissions';
  import { moderationError } from '../errors';
  import { chatStore } from '../../chats/chatStore.svelte';
  import { clock, shortStamp, stamp } from '../../shared/time';
  import { messengerApi, type MessengerGroup, type MessengerMessage, type QueueFilter, type QueueItem } from '../../api';
  import { actionsFor, confirmDecision, waitingOf, type Decision } from './queue';

  interface Props {
    group: MessengerGroup;
    author: string;
    /** The filter of the queue the card was found under. */
    filter: QueueFilter;
    onback: () => void;
  }
  let { group, author, filter, onback }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'msg_you', params);
  /** Rows drawn at a time: a flood of thousands is not one long page. */
  const STEP = 100;

  let items = $state<QueueItem[]>([]);
  let loaded = $state(false);
  let shown = $state(STEP);
  let busy = $state(false);
  let error = $state('');
  let selected = $state<string[]>([]);
  let open = $state<string | null>(null);
  let context = $state<Record<string, MessengerMessage[]>>({});

  const name = $derived(nameStore.label(author));
  const spamOk = $derived(canSpam(group, author));
  const allTicked = $derived(items.length > 0 && items.every((x) => selected.includes(x.message_id)));
  const ticked = $derived(items.filter((x) => selected.includes(x.message_id)));

  async function load() {
    const f = filter;
    error = '';
    try {
      const rows = await waitingOf(group.id, f, author);
      if (f !== filter) return;
      items = rows;
      loaded = true;
      const ids = new Set(rows.map((x) => x.message_id));
      selected = selected.filter((id) => ids.has(id));
      if (!rows.length) onback();
    } catch (e) {
      error = moderationError(e, tr);
    }
  }

  $effect(() => {
    void filter; void group.queue;
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
      for (const a of actionsFor(rows, kind)) await messengerApi.moderation.groupMod(group.id, a);
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
    const inChat = chatStore.activeId === chatId ? chatStore.messages : [];
    const at = inChat.findIndex((m) => m.id === row.message_id);
    let before = at > 0 ? inChat.slice(Math.max(0, at - 2), at) : [];
    if (at < 0) {
      try { before = (await messengerApi.chats.messages(chatId, row.at, 2)).slice(-2); }
      catch { before = []; }
    }
    context = { ...context, [row.message_id]: before.filter((m) => m.content_type !== 'system') };
  }
</script>

<div class="who">
  <button class="icon back" onclick={onback} title={$t('msg_back')} aria-label={$t('msg_back')}><Icon name="chevron-left" size={20} /></button>
  <Avatar url={nameStore.picture(author)} label={name} seed={author} size={36} />
  <div class="who-text">
    <span class="name">{name}</span>
    {#if loaded}<span class="count">{$t('msg_mod_queue_total', { n: String(items.length) })}</span>{/if}
  </div>
</div>

{#if error}<div class="error-msg">{error}</div>{/if}

{#if items.length}
  <div class="bulk">
    <label class="tick-all">
      <input type="checkbox" checked={allTicked} aria-label={$t('msg_mod_select_all')} onchange={() => (selected = allTicked ? [] : items.map((x) => x.message_id))} />
      <span>{ticked.length ? $t('msg_mod_selected', { n: String(ticked.length) }) : $t('msg_mod_select_all')}</span>
    </label>
  </div>
  <!-- The buttons know their width: the panel beside a chat is narrow. -->
  <div class="acts-box"><div class="acts">
    {#if ticked.length}
      <button class="btn btn-primary" disabled={busy} onclick={() => decide(ticked, 'approve')}><Icon name="check" size={16} />{$t('msg_mod_approve')}</button>
      <button class="btn btn-ghost" disabled={busy} onclick={() => decide(ticked, 'reject')}><Icon name="x" size={16} />{$t('msg_mod_reject')}</button>
    {:else}
      <button class="btn btn-primary" disabled={busy} onclick={() => decide(items, 'approve')}><Icon name="check" size={16} />{$t('msg_mod_approve_all')}</button>
      <button class="btn btn-ghost" disabled={busy} onclick={() => decide(items, 'reject')}><Icon name="x" size={16} />{$t('msg_mod_reject_all')}</button>
    {/if}
    {#if spamOk}
      <button class="btn btn-ghost spam" disabled={busy} title={$t('msg_mod_spam')} aria-label={$t('msg_mod_spam')} onclick={() => decide(items, 'spam')}><Icon name="ban" size={16} /></button>
    {/if}
  </div></div>

  <ul class="list divided">
    {#each items.slice(0, shown) as row (row.message_id)}
      <li class="msg" class:open={open === row.message_id}>
        <input class="tick" type="checkbox" checked={selected.includes(row.message_id)} onchange={() => tick(row.message_id)} aria-label={stamp(row.at)} />
        <div class="body">
          <button class="said-btn" onclick={() => expand(row)} aria-expanded={open === row.message_id}>
            <span class="when" title={stamp(row.at)}>{shortStamp(row.at)}</span>
            {#if open === row.message_id && context[row.message_id]?.length}
              <span class="context" aria-label={$t('msg_mod_context')}>
                {#each context[row.message_id] as c (c.id)}
                  <span class="said"><b>{nameStore.label(c.sender_pubkey)}</b> <span class="said-at">{clock(c.created_at)}</span> {c.text ?? ''}</span>
                {/each}
              </span>
            {/if}
          </button>
          <div class="text"><MessageContent text={row.text} cards={false} /></div>
        </div>
        <div class="row-acts">
          <button class="icon ok" disabled={busy} title={$t('msg_mod_approve')} aria-label={$t('msg_mod_approve')} onclick={() => decide([row], 'approve')}><Icon name="check" size={18} /></button>
          <button class="icon" disabled={busy} title={$t('msg_mod_reject')} aria-label={$t('msg_mod_reject')} onclick={() => decide([row], 'reject')}><Icon name="x" size={18} /></button>
        </div>
      </li>
    {/each}
  </ul>
  {#if items.length > shown}<button class="btn btn-ghost btn-sm more" onclick={() => (shown += STEP)}>{$t('msg_mod_more')}</button>{/if}
{:else if !loaded && !error}
  <div class="loading"><Icon name="loader" size={14} /></div>
{/if}

<style>
  .who { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  .back { margin-left: -6px; }
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
  .msg.open { background: var(--surface-row-hover); }
  .tick { margin-top: 3px; flex-shrink: 0; }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .said-btn { display: flex; flex-direction: column; align-items: flex-start; gap: 4px; border: none; background: none; padding: 0; color: inherit; font: inherit; cursor: pointer; text-align: left; }
  .when { font-size: var(--fs-2xs); color: var(--text-3); }
  .text { font-size: var(--fs-sm); color: var(--text-body); overflow-wrap: anywhere; }
  .context { display: flex; flex-direction: column; gap: 2px; padding: 2px 8px; border-left: 2px solid var(--border-2); }
  .said { font-size: var(--fs-xs); color: var(--text-3); overflow-wrap: anywhere; }
  .said-at { font-size: var(--fs-2xs); }
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

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->
<!-- Copied from ui/src/lib/messenger/groups/moderation/RequestsTab.svelte at 8ce09c39, then changed for channels. -->

<!--
  Who asks to be let into a channel by request, for its moderators: a card
  per request, newest first, a search once they are many, and answers for
  one or for the ticked. An answer for many runs one by one through the
  same commands as one answer, goes on past a failure and names, at the
  end, those it could not answer, with a way to try them again. The
  requests are read from the runtime (`channel_moderation_requests`) and
  again whenever the channel tells of one.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { countKey, locale, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { toast } from '$lib/core/ui/toast.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import RequestCard from './RequestCard.svelte';
  import RequestProfile from './RequestProfile.svelte';
  import { untrack } from 'svelte';
  import { confirmStore, contacts, writeTo } from '$lib/messenger/seams/shared';
  import { channelStore } from '../channelStore.svelte';
  import { nameStore, shortKey } from '../names.svelte';
  import { channelError } from '../errors';
  import { channelsApi, type ChannelDetail, type RequestView } from '../api';
  import {
    bulkTried, filterRequests, firstLine, keepSelected, runBulk, SEARCH_FROM, selectAll, sortRequests, toggleSelected,
    type BulkKind, type BulkState, type RequestFacts, type RequestOrder,
  } from './requests';

  interface Props {
    detail: ChannelDetail;
  }
  let { detail }: Props = $props();

  const cid = $derived(detail.channel.id);
  let requests = $state<RequestView[]>([]);

  async function loadRequests() {
    const id = cid;
    try {
      const list = await channelsApi.moderation.requests(id);
      if (id === cid) requests = (list ?? []).filter((r) => r.status === 'pending');
    } catch (e) {
      error = explain(e);
    }
  }

  // Again when the channel tells of a request, or of a decision on one.
  $effect(() => {
    void channelStore.requests[cid];
    void channelStore.journalRev[cid];
    untrack(() => loadRequests());
  });

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const explain = (e: unknown) => channelError(e, tr);

  const ORDERS: RequestOrder[] = ['newest', 'oldest'];
  let order = $state<RequestOrder>('newest');
  let query = $state('');
  let selected = $state<string[]>([]);
  /** The request whose answer is on its way. */
  let acting = $state<string | null>(null);
  let error = $state('');
  let bulk = $state<BulkState | null>(null);
  /** Names of those an act on many was for, kept: a request answered leaves the list. */
  let names = $state<Record<string, string>>({});
  let menu = $state<{ open: boolean; x: number; y: number; r: RequestView | null }>({ open: false, x: 0, y: 0, r: null });
  let profile = $state<{ open: boolean; r: RequestView | null }>({ open: false, r: null });

  /** Seconds now: «ждёт» moves on without anything else changing. */
  let now = $state(Math.floor(Date.now() / 1000));
  $effect(() => {
    const timer = setInterval(() => (now = Math.floor(Date.now() / 1000)), 30_000);
    return () => clearInterval(timer);
  });

  const running = $derived(!!bulk?.running);

  function factsOf(r: RequestView): RequestFacts {
    const pk = r.requester;
    const contact = contacts().find((c) => c.pubkey === pk) ?? null;
    const p = nameStore.profile(pk);
    const label = nameStore.label(pk);
    return {
      name: label || shortKey(pk),
      picture: nameStore.picture(pk),
      nip05: p?.nip05?.trim() || null,
      nip05Verified: !!p?.nip05_verified,
      contact: !!contact,
      talked: false,
      about: firstLine(p?.about),
      missing: nameStore.missing(pk),
    };
  }

  const facts = $derived(Object.fromEntries(requests.map((r) => [r.requester, factsOf(r)])) as Record<string, RequestFacts>);
  const sorted = $derived(sortRequests(requests, order));
  const shown = $derived(filterRequests(sorted, query, (r) => ({ name: facts[r.requester]?.name ?? '', nip05: facts[r.requester]?.nip05 ?? null })));
  const allShown = $derived(shown.length > 0 && shown.every((r) => selected.includes(r.requester)));
  const someShown = $derived(shown.some((r) => selected.includes(r.requester)));

  // A request answered (here, on another device of mine, by another
  // manager) is no longer ticked.
  $effect(() => {
    const kept = keepSelected(selected, requests);
    if (kept.length !== selected.length) selected = kept;
  });

  const nameOf = (who: string) => facts[who]?.name ?? names[who] ?? nameStore.label(who);

  const act = (kind: BulkKind, who: string) =>
    kind === 'ban' ? channelStore.act(cid, { op: 'ban', who }) : channelStore.answerRequest(cid, who, kind === 'approve');

  async function answer(r: RequestView, approve: boolean) {
    if (acting || running) return;
    acting = r.requester; error = '';
    try { await act(approve ? 'approve' : 'reject', r.requester); requests = requests.filter((x) => x.requester !== r.requester); }
    catch (e) { error = explain(e); }
    finally { acting = null; }
  }

  async function ban(r: RequestView) {
    if (acting || running) return;
    if (!(await confirmStore.ask($t('chan_req_ban_confirm', { name: nameOf(r.requester) }), $t('chan_req_ban'), true))) return;
    acting = r.requester; error = '';
    try { await act('ban', r.requester); requests = requests.filter((x) => x.requester !== r.requester); }
    catch (e) { error = explain(e); }
    finally { acting = null; }
  }

  const CONFIRM = { approve: 'chan_req_confirm_approve', reject: 'chan_req_confirm_reject', ban: 'chan_req_confirm_ban' } as const;
  const DONE = { approve: 'chan_req_done_approve', reject: 'chan_req_done_reject', ban: 'chan_req_done_ban' } as const;
  const PROGRESS = { approve: 'chan_req_progress_approve', reject: 'chan_req_progress_reject', ban: 'chan_req_progress_ban' } as const;
  const CTA = { approve: 'chan_req_approve', reject: 'chan_req_reject', ban: 'chan_req_ban' } as const;

  /** An answer for many: asked first (unless it is a retry of those that failed), then one by one. */
  async function many(kind: BulkKind, keys: string[], ask = true) {
    if (!keys.length || running || acting) return;
    const n = keys.length;
    if (ask && !(await confirmStore.ask($t(countKey(CONFIRM[kind], n, $locale), { n: String(n) }), $t(CTA[kind]), kind !== 'approve'))) return;
    names = { ...names, ...Object.fromEntries(keys.map((k) => [k, nameOf(k)])) };
    selected = selected.filter((k) => !keys.includes(k));
    error = '';
    const end = await runBulk(kind, keys, (who) => act(kind, who), explain, (s) => (bulk = s));
    await loadRequests();
    if (!end.failed.length) {
      bulk = null;
      toast.success($t(countKey(DONE[kind], end.done, $locale), { n: String(end.done) }));
    }
  }

  const failedNames = $derived(bulk && !bulk.running ? bulk.failed.map((f) => nameOf(f.requester)).join(', ') : '');
  /** Why they failed, when it is one reason for all of them. */
  const failedWhy = $derived(bulk && !bulk.running && new Set(bulk.failed.map((f) => f.error)).size === 1 ? bulk.failed[0].error : '');

  function entries(r: RequestView): MenuEntry[] {
    const list: MenuEntry[] = [{ label: $t('chan_req_profile'), icon: 'user', onselect: () => (profile = { open: true, r }) }];
    if (facts[r.requester]?.contact) {
      list.push({ label: $t('chan_menu_write'), icon: 'message-circle', onselect: () => { writeTo(r.requester).catch(() => {}); } });
    }
    list.push({ type: 'separator' }, { label: $t('chan_req_ban'), icon: 'user-cancel', danger: true, onselect: () => ban(r) });
    return list;
  }
  const items = $derived(menu.r ? entries(menu.r) : []);
</script>

{#if bulk && !bulk.running && bulk.failed.length}
  <div class="status failed" role="alert">
    <Icon name="alert-triangle" size={14} />
    <div class="status-text">
      <span>{$t('chan_req_failed', { names: failedNames })}</span>
      {#if failedWhy}<span class="why">{failedWhy}</span>{/if}
      <button type="button" class="btn btn-ghost btn-sm retry" onclick={() => bulk && many(bulk.kind, bulk.failed.map((f) => f.requester), false)}>
        <Icon name="refresh-cw" size={13} />{$t('chan_retry')}
      </button>
    </div>
    <button type="button" class="icon-btn" title={$t('chan_close')} aria-label={$t('chan_close')} onclick={() => (bulk = null)}><Icon name="x" size={14} /></button>
  </div>
{/if}

{#if error}<div class="error-msg">{error}</div>{/if}

{#if requests.length}
  <div class="tools">
    {#if requests.length > SEARCH_FROM || query}
      <div class="search">
        <Icon name="search" size={13} />
        <input type="search" bind:value={query} placeholder={$t('chan_req_search')} aria-label={$t('chan_req_search')} />
      </div>
    {/if}
    <div class="row">
      <label class="tick-all">
        <input type="checkbox" checked={allShown} indeterminate={someShown && !allShown} disabled={running || !shown.length}
          onchange={() => (selected = selectAll(selected, shown))} />
        <span>{$t('chan_mod_select_all')}</span>
      </label>
      <div class="seg" role="radiogroup" aria-label={$t('chan_req_sort')}>
        {#each ORDERS as o (o)}
          <button class="seg-btn" class:active={order === o} role="radio" aria-checked={order === o} onclick={() => (order = o)}>
            {$t(`chan_req_sort_${o}` as 'chan_req_sort_newest')}
          </button>
        {/each}
      </div>
    </div>
  </div>

  {#if shown.length}
    <ul class="cards">
      {#each shown as r (r.requester)}
        <RequestCard request={r} facts={facts[r.requester]} {now} selected={selected.includes(r.requester)} busy={running || acting === r.requester}
          ontick={() => (selected = toggleSelected(selected, r.requester))}
          onanswer={(approve) => answer(r, approve)}
          onmenu={(at) => (menu = { open: true, x: at.x, y: at.y, r })} />
      {/each}
    </ul>
  {:else}
    <p class="nothing">{$t('chan_req_nothing_found')}</p>
  {/if}
{:else if !bulk?.running}
  <div class="empty-state">
    <div class="empty-icon"><Icon name="user-plus" size={24} /></div>
    <p>{$t('chan_req_empty')}</p>
    <span>{$t('chan_req_empty_hint')}</span>
  </div>
{/if}

{#if running && bulk}
  <div class="bar" role="status">
    <div class="progress">
      <span>{$t(PROGRESS[bulk.kind], { done: String(bulkTried(bulk)), total: String(bulk.total) })}</span>
      <span class="track"><span class="fill" style:width={`${(bulkTried(bulk) / Math.max(1, bulk.total)) * 100}%`}></span></span>
    </div>
  </div>
{:else if selected.length}
  <div class="bar">
    <div class="bar-head">
      <span class="picked">{$t('chan_mod_selected', { n: String(selected.length) })}</span>
      <button type="button" class="icon-btn" title={$t('chan_req_unselect')} aria-label={$t('chan_req_unselect')} onclick={() => (selected = [])}><Icon name="x" size={14} /></button>
    </div>
    <div class="bar-acts">
      <button type="button" class="btn btn-primary btn-sm" onclick={() => many('approve', [...selected])}><Icon name="check" size={14} />{$t('chan_req_approve')}</button>
      <button type="button" class="btn btn-ghost btn-sm" onclick={() => many('reject', [...selected])}>{$t('chan_req_reject')}</button>
      <!-- The ban is a mark with its name in the tooltip: three words do not fit a panel's width. -->
      <button type="button" class="btn btn-ghost btn-sm danger ban" title={$t('chan_req_ban_short')} aria-label={$t('chan_req_ban_short')} onclick={() => many('ban', [...selected])}><Icon name="user-cancel" size={15} /></button>
    </div>
  </div>
{/if}

<ContextMenu bind:open={menu.open} x={menu.x} y={menu.y} {items} onclose={() => (menu.open = false)} />
<RequestProfile bind:open={profile.open} request={profile.r} facts={profile.r ? (facts[profile.r.requester] ?? null) : null} />

<style>
  .tools { display: flex; flex-direction: column; gap: var(--sp-2); }
  .search {
    display: flex; align-items: center; gap: 6px; padding: 0 12px; color: var(--text-3);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-pill);
  }
  .search:focus-within { border-color: var(--accent-border); }
  .search input { flex: 1; min-width: 0; border: none; background: none; font: inherit; font-size: var(--fs-sm); color: var(--text); padding: 7px 0; outline: none; }
  .row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); flex-wrap: wrap; }
  .tick-all { display: flex; align-items: center; gap: var(--sp-2); padding-left: var(--sp-3); font-size: var(--fs-xs); color: var(--text-2); cursor: pointer; user-select: none; }
  .seg-btn { height: 28px; padding: 0 var(--sp-3); font-size: var(--fs-xs); }
  .cards { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .nothing { margin: 0; font-size: var(--fs-sm); color: var(--text-3); text-align: center; padding: var(--sp-4) 0; }
  .empty-state { padding: var(--sp-10) var(--sp-4); }
  .empty-state p { margin: 0; }
  .status {
    display: flex; align-items: flex-start; gap: var(--sp-2); padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-3); border-radius: var(--radius-md);
    font-size: var(--fs-xs); line-height: 1.4;
  }
  .status.failed { color: var(--danger-text); background: var(--danger-bg); border: 1px solid var(--danger-border); }
  .status > :global(svg) { flex-shrink: 0; margin-top: 2px; }
  .status-text { flex: 1; min-width: 0; overflow-wrap: anywhere; display: flex; flex-direction: column; align-items: flex-start; gap: 4px; padding-top: 1px; }
  .retry { margin-top: 2px; }
  .why { color: var(--text-2); }
  /* The answers for the ticked float over the cards at the foot of the panel. */
  .bar {
    position: sticky; bottom: 0; z-index: 1; margin-top: auto;
    display: flex; flex-direction: column; gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-3); margin-bottom: var(--sab, 0px);
    background: var(--glass-bar); border: 1px solid var(--glass-border); border-radius: var(--radius-lg);
    box-shadow: var(--pane-shadow), inset 0 1px 0 var(--glass-highlight);
    backdrop-filter: var(--glass-filter); -webkit-backdrop-filter: var(--glass-filter);
    animation: vslide var(--dur-base) var(--ease);
  }
  .picked { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); }
  .bar-head { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); }
  .bar-acts { display: flex; align-items: center; gap: 6px; }
  .bar-acts .btn { flex: 1 1 0; justify-content: center; min-width: 0; }
  .bar-acts .btn.ban { flex: 0 0 auto; }
  .danger { color: var(--danger-text); }
  .progress { flex: 1; display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-sm); font-weight: var(--fw-semibold); padding: 4px 0; }
  .track { height: 4px; border-radius: var(--radius-pill); background: var(--surface-3); overflow: hidden; }
  .fill { display: block; height: 100%; background: var(--accent); border-radius: inherit; transition: width var(--dur-base) var(--ease); }
  @media (pointer: coarse) {
    .search input { font-size: var(--fs-md); min-height: 40px; }
    .bar-acts .btn { min-height: var(--control-h); }
  }
  @media (prefers-reduced-motion: reduce) { .bar { animation: none; } }
</style>

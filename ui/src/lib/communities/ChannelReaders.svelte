<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The readers of a channel, for its team only (the owner's decision,
  2026-10-25; `channel_readers`): newest first, a hundred at a time, found
  by a name among those loaded or by the start of a key among them all.
  A row opens the menu of the person (`onmenu`, the team's own: write,
  roles, levels, mute, remove, ban); the list is read again with every
  change of the journal or of the keeper. Readers never see it.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { Avatar, fold, longpress, matchesQuery, stamp } from '$lib/messenger/seams/shared';
  import { channelStore } from './channelStore.svelte';
  import { channelError } from './errors';
  import { nameStore, shortKey } from './names.svelte';
  import { channelsApi, type ChannelDetail, type ReaderView, type StaffView } from './api';
  import { mutedNow, staffOf } from './permissions';
  import { levelNames, levelsOf } from './levels/sections';
  import { PARTIAL_RETRIES, PARTIAL_RETRY_MS, READERS_MAX_PAGE, READERS_PAGE, appendPage, keyQuery, shownReaders } from './readers';

  interface Props {
    detail: ChannelDetail;
    /** The menu of a person, as the team list opens it. */
    onmenu: (e: MouseEvent | { x: number; y: number }, s: StaffView) => void;
    /** Whether that menu has anything for the person. */
    hasMenu: (s: StaffView) => boolean;
  }
  let { detail, onmenu, hasMenu }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const cid = $derived(detail.channel.id);

  let rows = $state<ReaderView[]>([]);
  let total = $state(0);
  let partial = $state(false);
  let loading = $state(false);
  let error = $state('');
  let query = $state('');
  /** What the core found by the start of a key, for the query typed. */
  let hits = $state<ReaderView[]>([]);
  /** Reads of the first rows done: a list still partial is asked again after each. */
  let reads = $state(0);

  /** One read of the list, its first rows again: as many as are shown (a refresh after an act keeps the place). */
  async function load(id: string, count: number) {
    loading = true;
    try {
      const page = await channelsApi.moderation.readers(id, 0, Math.min(READERS_MAX_PAGE, Math.max(READERS_PAGE, count)));
      if (id !== cid) return;
      rows = page.readers; total = page.total; partial = page.partial; error = '';
      reads += 1;
    } catch (e) {
      if (id === cid) error = channelError(e, tr);
    } finally {
      loading = false;
    }
  }

  async function more() {
    const id = cid;
    loading = true;
    try {
      const page = await channelsApi.moderation.readers(id, rows.length, READERS_PAGE);
      if (id !== cid) return;
      rows = appendPage(rows, page.readers); total = page.total; partial = page.partial;
    } catch (e) {
      error = channelError(e, tr);
    } finally {
      loading = false;
    }
  }

  // Read again with news of the journal (a role, a ban, a removal) and of the keeper (who came, who left).
  $effect(() => {
    const id = cid;
    void channelStore.journalRev[id];
    void channelStore.keeperRev[id];
    untrack(() => load(id, rows.length));
  });

  // Shards of the snapshot on their way: asked again a few times after each read, until the list is whole.
  let retries = 0;
  $effect(() => {
    void reads;
    if (!partial) { retries = 0; return; }
    if (retries >= PARTIAL_RETRIES) return;
    const id = cid;
    const timer = setTimeout(() => { retries += 1; untrack(() => load(id, rows.length)); }, PARTIAL_RETRY_MS);
    return () => clearTimeout(timer);
  });

  // The start of a key is looked for among all the readers, a little after the typing stops.
  const typedKey = $derived(keyQuery(query));
  $effect(() => {
    const id = cid;
    const key = typedKey;
    if (!key) { hits = []; return; }
    const timer = setTimeout(() => {
      channelsApi.moderation.readers(id, 0, READERS_PAGE, key).then((p) => { if (id === cid && key === typedKey) hits = p.readers; }).catch(() => {});
    }, 250);
    return () => clearTimeout(timer);
  });

  /** Every name and key of a reader, folded, as the page knows them. */
  function haystack(who: string): string {
    const p = nameStore.profile(who);
    return fold([nameStore.label(who), p?.display_name, p?.name, p?.nip05, who].filter(Boolean).join('\n'));
  }

  const shown = $derived(shownReaders(rows, hits, query, (w) => matchesQuery(haystack(w), query)));
  const lv = $derived(levelsOf(detail));
  const chips = (r: ReaderView) => levelNames(lv.levels, [0, ...r.levels]);
  const searching = $derived(!!query.trim());
</script>

<section class="readers" aria-label={$t('chan_readers')}>
  <div class="label">{$t('chan_readers')} · {total}</div>
  {#if total > 0 || searching}
    <div class="search">
      <Icon name="search" size={14} />
      <input type="search" bind:value={query} placeholder={$t('chan_readers_search')} aria-label={$t('chan_readers_search')} spellcheck="false" autocomplete="off" />
    </div>
  {/if}
  {#if error}<div class="error-msg" role="alert">{error}</div>{/if}

  {#if shown.length}
    <ul class="list divided">
      {#each shown as r (r.pubkey)}
        {@const s = staffOf(detail, r.pubkey)}
        {@const nm = nameStore.label(r.pubkey)}
        <li>
          <button class="person" class:still={!hasMenu(s)} title={$t('chan_readers_since', { date: stamp(r.since) })}
            oncontextmenu={(e) => onmenu(e, s)} onclick={(e) => onmenu(e, s)} use:longpress={{ onpress: (p) => onmenu(p, s) }}>
            <Avatar url={nameStore.picture(r.pubkey)} label={nm} seed={r.pubkey} size={34} />
            <span class="who">
              <span class="who-name">{nm}</span>
              <code>{shortKey(r.pubkey)}</code>
              {#if chips(r).length}
                <span class="lvls" title={$t('chan_levels')}>{#each chips(r) as n (n)}<span class="lvl"><Icon name="layers" size={10} />{n}</span>{/each}</span>
              {/if}
            </span>
            {#if mutedNow(s)}<span class="dim" title={$t('chan_member_muted')}><Icon name="bell-off" size={13} /></span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {:else if !loading}
    <p class="hint">{searching ? $t('chan_readers_nothing') : $t('chan_readers_none')}</p>
  {/if}

  {#if !searching && rows.length < total}
    <button class="btn btn-ghost btn-sm more" disabled={loading} aria-busy={loading} onclick={more}>
      {#if loading}<span class="spinner" aria-hidden="true"></span>{/if}{$t('chan_readers_more')}
    </button>
  {/if}
  {#if partial}<p class="hint"><Icon name="loader" size={12} /> {$t('chan_readers_partial')}</p>{/if}
  <p class="hint">{$t('chan_readers_team_hint')}</p>
</section>

<style>
  .readers { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; }
  .label { font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.6px; color: var(--text-3); font-weight: var(--fw-bold); }
  .search { position: relative; display: flex; align-items: center; color: var(--text-3); }
  .search :global(svg) { position: absolute; left: 10px; pointer-events: none; }
  .search input {
    width: 100%; font: inherit; font-size: var(--fs-sm); color: var(--text); background: var(--surface-2);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: 8px 10px 8px 32px;
  }
  .search input:focus { outline: none; border-color: var(--accent-border); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; --rows-gap: 2px; }
  .person {
    display: flex; align-items: center; gap: var(--sp-3); padding: 6px 8px; border-radius: var(--radius-sm); min-width: 0;
    width: 100%; border: none; background: none; color: inherit; font: inherit; text-align: left; cursor: pointer;
  }
  .person:hover { background: var(--surface-row-hover); }
  .person.still { cursor: default; }
  .person.still:hover { background: none; }
  .who { display: flex; flex-direction: column; gap: 1px; min-width: 0; flex: 1; }
  .who-name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .who code { font-family: var(--font-mono); font-size: var(--fs-2xs); color: var(--text-3); }
  .lvls { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 2px; }
  .lvl {
    display: inline-flex; align-items: center; gap: 3px; max-width: 100%; padding: 0 7px; border-radius: var(--radius-pill);
    font-size: var(--fs-2xs); font-weight: var(--fw-semibold); color: var(--accent-text-2); background: var(--accent-tint);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .dim { color: var(--text-3); display: inline-flex; }
  .more { align-self: flex-start; }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; display: flex; align-items: center; gap: 4px; }
  /* On the desktop a long list scrolls in itself, the panel's other blocks stay in reach; on the phone it takes the sheet. */
  @media (pointer: fine) {
    .list { max-height: 420px; overflow-y: auto; }
  }
  @media (pointer: coarse) {
    .search input { font-size: var(--fs-md); min-height: 44px; }
    .person { padding: 9px 8px; }
  }
</style>

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
  <div class="block-head"><span class="cap">{$t('chan_readers')}<b class="count-pill quiet">{total}</b></span></div>
  {#if total > 0 || searching}
    <label class="search">
      <Icon name="search" size={16} />
      <input type="search" bind:value={query} placeholder={$t('chan_readers_search')} aria-label={$t('chan_readers_search')} spellcheck="false" autocomplete="off" />
    </label>
  {/if}
  {#if error}<div class="error-msg" role="alert">{error}</div>{/if}

  {#snippet moreButton()}
    <button class="more" disabled={loading} aria-busy={loading} onclick={more}>
      {#if loading}<span class="spinner" aria-hidden="true"></span>{/if}{$t('chan_readers_more')}
    </button>
  {/snippet}

  <!-- The rows are the team's, in one card: on a computer the list scrolls inside it, «Показать ещё» its foot. -->
  {#if shown.length}
    <div class="rows-card">
      <ul class="list">
        {#each shown as r (r.pubkey)}
          {@const s = staffOf(detail, r.pubkey)}
          {@const nm = nameStore.label(r.pubkey)}
          <li>
            <button class="person" class:still={!hasMenu(s)} title={$t('chan_readers_since', { date: stamp(r.since) })}
              oncontextmenu={(e) => onmenu(e, s)} onclick={(e) => onmenu(e, s)} use:longpress={{ onpress: (p) => onmenu(p, s) }}>
              <Avatar url={nameStore.picture(r.pubkey)} label={nm} seed={r.pubkey} size={36} />
              <span class="who">
                <span class="who-name">{nm}</span>
                <code>{shortKey(r.pubkey)}</code>
                {#if chips(r).length}
                  <span class="lvls" title={$t('chan_levels')}>{#each chips(r) as n (n)}<span class="lvl"><Icon name="layers" size={14} />{n}</span>{/each}</span>
                {/if}
              </span>
              {#if mutedNow(s)}<span class="dim" title={$t('chan_member_muted')}><Icon name="bell-off" size={16} /></span>{/if}
            </button>
          </li>
        {/each}
      </ul>
      {#if !searching && rows.length < total}{@render moreButton()}{/if}
    </div>
  {:else}
    <!-- Kept through a read again (every change of the journal or the keeper): a line gone and back at the bottom of the panel shook it. -->
    {#if !loading || reads > 0}<p class="hint">{searching ? $t('chan_readers_nothing') : $t('chan_readers_none')}</p>{/if}
    {#if !searching && rows.length < total}<div class="rows-card">{@render moreButton()}</div>{/if}
  {/if}

  {#if partial}<p class="hint with-icon"><Icon name="loader" size={14} /> {$t('chan_readers_partial')}</p>{/if}
  <p class="hint">{$t('chan_readers_team_hint')}</p>
</section>

<style>
  /* The block as the team's (mockup «Каналы · читатели», rd.js): a word and its count, the panel's field,
     the team's rows in one card, the notes under it. */
  .readers { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; }
  .block-head { display: flex; align-items: center; min-height: 24px; margin-bottom: 2px; }
  .cap { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); }
  .search { position: relative; display: flex; align-items: center; color: var(--text-3); }
  .search :global(svg) { position: absolute; left: var(--sp-3); pointer-events: none; }
  .search input { height: var(--control-h); padding: 0 var(--sp-3) 0 36px; font-size: var(--fs-base); }
  .rows-card { display: flex; flex-direction: column; min-width: 0; overflow: hidden; border-radius: var(--radius-lg); border: 1px solid var(--border); background: var(--surface-2); }
  .list { list-style: none; margin: 0; padding: var(--sp-1); display: flex; flex-direction: column; min-width: 0; }
  /* A hairline between two rows, as in the team's box; hidden around a hovered row. */
  .list > li { position: relative; }
  .list > li + li::before {
    content: ''; position: absolute; top: 0; left: var(--sp-3); right: var(--sp-3); height: 1px; background: var(--border-2);
    pointer-events: none; transition: opacity var(--dur-fast) var(--ease);
  }
  .list > li:has(> .person:not(.still):hover)::before, .list > li:has(> .person:not(.still):hover) + li::before { opacity: 0; }
  .person {
    display: flex; align-items: center; gap: var(--sp-3); min-height: 52px; padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-3);
    border-radius: var(--radius-md); min-width: 0; width: 100%; border: none; background: none; color: inherit; font: inherit;
    text-align: left; cursor: pointer; transition: background var(--dur-fast) var(--ease);
  }
  .person:hover { background: var(--surface-row-hover); }
  .person.still { cursor: default; }
  .person.still:hover { background: none; }
  .who { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .who-name { font-size: var(--fs-base); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .who code { font-family: var(--font-mono); font-size: var(--fs-2xs); color: var(--text-3); }
  .lvls { display: flex; flex-wrap: wrap; gap: var(--sp-1); margin-top: 2px; }
  .lvl {
    display: inline-flex; align-items: center; gap: var(--sp-1); max-width: 100%; height: 20px; padding: 0 var(--sp-2); border-radius: var(--radius-pill);
    font-size: var(--fs-2xs); font-weight: var(--fw-semibold); color: var(--accent-text); background: var(--accent-bg);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .dim { flex: none; color: var(--text-3); display: inline-flex; }
  /* «Показать ещё»: the foot of the card, accent text across its width. */
  .more {
    flex: none; display: flex; align-items: center; justify-content: center; gap: var(--sp-2); width: 100%; min-height: 44px;
    border: none; border-top: 1px solid var(--border-2); background: none; color: var(--accent-text); font: inherit;
    font-size: var(--fs-base); font-weight: var(--fw-bold); cursor: pointer; transition: background var(--dur-fast) var(--ease);
  }
  .more:first-child { border-top: none; }
  .more:hover:not(:disabled) { background: var(--surface-row-hover); }
  .more:disabled { color: var(--text-3); cursor: default; }
  .more:focus-visible { outline: 2px solid var(--accent-border); outline-offset: -2px; }
  .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-3); line-height: 1.45; }
  .hint.with-icon { display: flex; align-items: center; gap: var(--sp-1); }
  /* On the desktop a long list scrolls inside its card, the field and the foot stay in reach; on the phone it takes the page. */
  @media (pointer: fine) {
    .list { max-height: 420px; overflow-y: auto; }
  }
  @media (pointer: coarse) {
    .search input { height: var(--control-h-lg); font-size: var(--fs-md); }
    .person { min-height: 56px; }
    .more { min-height: 48px; }
  }
</style>

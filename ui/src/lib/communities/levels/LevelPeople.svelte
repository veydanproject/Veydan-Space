<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The people of one level, in its dialog (communities-wire §14.1,
  `set_levels`): who holds it, found by name or key, taken out with a
  button; and, for what is typed, those the page knows (the channel's
  journal, my contacts) or a hex key, given it with another. Each change
  goes to the journal at once, as in a person's own dialog of levels.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { Avatar, contactHaystacks, contacts as contactsOf, fold, matchesQuery, myKey } from '$lib/messenger/seams/shared';
  import { channelStore } from '../channelStore.svelte';
  import { channelError } from '../errors';
  import { nameStore, shortKey } from '../names.svelte';
  import { canSetLevels } from '../permissions';
  import { channelsApi, type ChannelDetail } from '../api';
  import { candidates, holders, isHexKey, levelsWith } from './people';
  import type { LevelView } from './types';

  interface Props {
    detail: ChannelDetail;
    level: LevelView;
  }
  let { detail, level }: Props = $props();

  /** How many people a search lists to add: the rest is found by typing more. */
  const MAX_FOUND = 30;

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const cid = $derived(detail.channel.id);
  const me = $derived(myKey());

  let query = $state('');
  /** The person whose change is on its way. */
  let busy = $state('');
  let error = $state('');

  /** The banned of the channel: given no level (a moderator's list). */
  let banned = $state<Set<string>>(new Set());
  $effect(() => {
    const id = cid;
    channelsApi.moderation.blocked(id).then((b) => { if (id === cid) banned = new Set(b); }).catch(() => {});
  });

  const all = $derived(contactsOf());
  const contactHay = $derived(contactHaystacks(all));
  /** Every name and key of a person, folded: a contact's own, else the profile the page knows. */
  function haystack(who: string): string {
    const own = contactHay.get(who);
    if (own) return own;
    const p = nameStore.profile(who);
    return fold([nameStore.label(who), p?.display_name, p?.name, p?.nip05, who].filter(Boolean).join('\n'));
  }

  const byName = (a: string, b: string) => nameStore.label(a).localeCompare(nameStore.label(b));
  const held = $derived(holders(detail, level.id).sort(byName));
  const shownHeld = $derived(held.filter((w) => matchesQuery(haystack(w), query)));
  const typedKey = $derived(isHexKey(query) ? query.trim().toLowerCase() : '');
  /** Those to add, for what is typed: nothing while the field is empty. */
  const found = $derived.by(() => {
    if (!query.trim()) return [];
    const known = [...detail.staff.map((s) => s.pubkey), ...all.map((c) => c.pubkey)];
    const list = candidates(known, held, banned, (w) => canSetLevels(detail, w, me)).filter((w) => w === typedKey || matchesQuery(haystack(w), query));
    if (typedKey && !list.includes(typedKey) && !held.includes(typedKey) && !banned.has(typedKey) && canSetLevels(detail, typedKey, me)) list.unshift(typedKey);
    return list.slice(0, MAX_FOUND);
  });

  async function change(who: string, give: boolean) {
    busy = who; error = '';
    try {
      await channelStore.setMemberLevels(cid, who, levelsWith(detail, who, level.id, give));
      if (give && who === typedKey) query = '';
    } catch (e) {
      error = channelError(e, tr);
    } finally {
      busy = '';
    }
  }
</script>

<section class="people" aria-label={$t('chan_level_people')}>
  <span class="label">{$t('chan_level_people')} · {held.length}</span>
  <!-- The search and the people in one card: the field its head, the rows under a line. -->
  <div class="box">
  <div class="search">
    <Icon name="search" size={16} />
    <input type="search" bind:value={query} placeholder={$t('chan_level_people_search')} aria-label={$t('chan_level_people_search')} spellcheck="false" autocomplete="off"
      onkeydown={(e) => { if (e.key === 'Enter' && found.length === 1 && !busy) { e.preventDefault(); change(found[0], true); } }} />
  </div>
  {#if error}<div class="error-msg" role="alert">{error}</div>{/if}

  <div class="rows">
  {#snippet person(who: string)}
    {@const nm = nameStore.label(who)}
    <Avatar url={nameStore.picture(who)} label={nm} seed={who} size={32} />
    <span class="who">
      <span class="name">{nm}{#if who === me} <span class="dim">({$t('chan_you')})</span>{/if}</span>
      <code>{shortKey(who)}</code>
    </span>
  {/snippet}

  {#if found.length}
    <span class="sub-label">{$t('chan_level_people_add_to')}</span>
    <ul class="list divided">
      {#each found as who (who)}
        <li class="row">
          {@render person(who)}
          <button type="button" class="btn btn-ghost btn-sm" disabled={!!busy} aria-busy={busy === who} onclick={() => change(who, true)}>
            {#if busy === who}<span class="spinner" aria-hidden="true"></span>{:else}<Icon name="plus" size={18} />{/if}{$t('chan_level_people_add')}
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  {#if shownHeld.length}
    {#if found.length}<span class="sub-label">{$t('chan_level_people_in')}</span>{/if}
    <ul class="list divided">
      {#each shownHeld as who (who)}
        <li class="row">
          {@render person(who)}
          {#if canSetLevels(detail, who, me)}
            <button type="button" class="icon danger" disabled={!!busy} aria-busy={busy === who} onclick={() => change(who, false)}
              title={$t('chan_level_people_remove')} aria-label={$t('chan_level_people_remove')}>
              {#if busy === who}<span class="spinner" aria-hidden="true"></span>{:else}<Icon name="x" size={16} />{/if}
            </button>
          {/if}
        </li>
      {/each}
    </ul>
  {:else if !found.length}
    <p class="hint empty">{held.length || query.trim() ? $t('chan_level_people_nothing') : $t('chan_level_people_none')}</p>
  {/if}
  </div>
  </div>
  <p class="hint">{$t('chan_level_people_hint')}</p>
</section>

<style>
  .people { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-bold); color: var(--text-2); }
  /* The words over a part of the list in sentence case. */
  .sub-label { font-size: var(--fs-xs); color: var(--text-3); font-weight: var(--fw-bold); margin-top: 2px; }
  /* One card: the search 44 as its head, the people under a line. */
  .box { display: flex; flex-direction: column; min-width: 0; border-radius: var(--radius-lg); border: 1px solid var(--border); background: var(--surface-2); overflow: hidden; }
  .box:has(.search input:focus-visible) { border-color: var(--accent-border); }
  .search { position: relative; display: flex; align-items: center; color: var(--text-3); border-bottom: 1px solid var(--border-2); }
  .search :global(svg) { position: absolute; left: var(--sp-3); pointer-events: none; }
  .search input {
    width: 100%; height: var(--control-h-lg); box-sizing: border-box; font: inherit; font-size: var(--fs-sm); color: var(--text); background: none;
    border: 0; border-radius: 0; padding: 0 var(--sp-3) 0 36px;
  }
  .search input:focus { outline: none; box-shadow: none; }
  .box .error-msg { margin: var(--sp-2) var(--sp-3) 0; }
  .rows { display: flex; flex-direction: column; min-width: 0; }
  .rows .sub-label { padding: var(--sp-2) var(--sp-3) 0; }
  .empty { padding: var(--sp-3); }
  /* The people: rows inside the card. */
  .list { list-style: none; margin: 0; padding: 0 var(--sp-3); display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; gap: var(--sp-3); min-height: 52px; padding: var(--sp-2) 0; min-width: 0; }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .name { font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dim { color: var(--text-3); font-weight: var(--fw-normal); }
  code { font-size: var(--fs-2xs); color: var(--text-3); }
  .btn { flex-shrink: 0; }
  .icon { flex-shrink: 0; border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; align-items: center; justify-content: center; width: 36px; height: 36px; padding: 0; border-radius: var(--radius); }
  .icon:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .icon:hover:not(:disabled) { background: var(--surface-3); }
  .icon.danger { color: var(--danger-text); }
  .icon:disabled, .btn:disabled { opacity: 0.55; cursor: default; }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  /* The desktop's dialog keeps its footer in sight: a long list scrolls in itself. */
  @media (pointer: fine) {
    .rows { max-height: 264px; overflow-y: auto; }
  }
  @media (pointer: coarse) {
    .search input { font-size: var(--fs-md); height: 44px; }
    .icon { width: 44px; height: 44px; }
  }
</style>

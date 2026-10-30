<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The levels and sections of a channel, for its managers (communities-wire
  §14.1, spec §5): each level with its name, the sections it opens, its
  rights and how many have it; each section with the levels that open it.
  A level is made and changed in its dialog; a section is added by an
  admin and deleted by the owner, after a question that names how many
  levels lose it. Everyone's section and the base level always stay.
  The info of a channel shows each on a page of its own (`part`).
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { countKey, locale, localeTag, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { confirmStore } from '$lib/messenger/seams/shared';
  import { channelStore } from '../channelStore.svelte';
  import { channelError } from '../errors';
  import { canSetUpSections } from '../permissions';
  import type { ChannelDetail } from '../api';
  import { BASE_LEVEL, EVERYONE, levelsOf } from './sections';
  import { MAX_LEVELS, MAX_SCOPES, RIGHTS, grantingCount, orderedLevels } from './editor';
  import LevelDialog from './LevelDialog.svelte';
  import ScopeDialog from './ScopeDialog.svelte';
  import SectionBadge from './SectionBadge.svelte';
  import { ordered } from './look';
  import type { LevelView, ScopeView } from './types';
  import Box from '../info/Box.svelte';
  import Tile from '$lib/core/ui/Tile.svelte';
  import Chev from '$lib/core/ui/Chev.svelte';

  interface Props {
    detail: ChannelDetail;
    /** Which of the two it shows: the levels, the sections, or both. */
    part?: 'levels' | 'sections' | 'all';
  }
  let { detail, part = 'all' }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const cid = $derived(detail.channel.id);
  const lv = $derived(levelsOf(detail));
  const levels = $derived(orderedLevels(lv.levels));
  /** The sections in the owner's order, as the tiles show them. */
  const scopes = $derived(ordered(lv.scopes));

  let editing = $state<{ open: boolean; level: LevelView | null }>({ open: false, level: null });
  /** The dialog of a section: a new one (`null`) or the settings of one (the owner). */
  let scopeDialog = $state<{ open: boolean; id: string | null }>({ open: false, id: null });
  let error = $state('');
  let busy = $state(false);

  const fmt = (n: number) => new Intl.NumberFormat(localeTag($locale)).format(n);
  const titleOf = (s: ScopeView | undefined) => (!s ? '' : s.scope_id === EVERYONE && !s.title ? $t('chan_scope_everyone') : s.title);

  /** The sections a level opens, in words; the base level's is everyone's. */
  function opens(l: LevelView): string {
    const own = l.scopes.filter((s) => s !== EVERYONE).map((id) => titleOf(lv.scopes.find((s) => s.scope_id === id))).filter(Boolean);
    if (l.id === BASE_LEVEL) return [titleOf(lv.scopes.find((s) => s.scope_id === EVERYONE)) || $t('chan_scope_everyone'), ...own].join(', ');
    return own.length ? own.join(', ') : $t('chan_level_no_scopes');
  }

  const rights = (l: LevelView) => RIGHTS.filter((r) => l.can[r]).map((r) => $t(`chan_right_${r}_short` as 'chan_right_comment_short'));

  async function removeScope(s: ScopeView) {
    const n = grantingCount(lv.levels, s.scope_id);
    const text = n
      ? $t(countKey('chan_scope_delete_confirm', n, $locale), { title: s.title, n: String(n) })
      : $t('chan_scope_delete_confirm_none', { title: s.title });
    if (!(await confirmStore.ask(text, $t('chan_scope_delete'), true))) return;
    busy = true; error = '';
    try { await channelStore.delScope(cid, s.scope_id); }
    catch (e) { error = channelError(e, tr); }
    finally { busy = false; }
  }
</script>

{#if part !== 'sections'}
<section class="block" aria-label={$t('chan_levels')}>
  <Box tag="ul">
    {#each levels as l (l.id)}
      <li>
        <button class="row as-button" onclick={() => (editing = { open: true, level: l })}>
          <Tile icon={l.id === BASE_LEVEL ? 'users' : 'layers'} tone={l.id === BASE_LEVEL ? 'accent' : 'plain'} />
          <span class="row-text">
            <span class="row-name">{l.name}{#if l.id === BASE_LEVEL}<span class="tag">{$t('chan_level_base')}</span>{/if}</span>
            <span class="row-sub"><Icon name="folder" size={14} />{opens(l)}</span>
            {#if rights(l).length}<span class="row-sub"><Icon name="pencil" size={14} />{rights(l).join(' · ')}</span>{/if}
          </span>
          <span class="members" title={$t(countKey('chan_level_members', l.members, $locale), { n: fmt(l.members) })}><Icon name="user" size={14} />{fmt(l.members)}</span>
          <Chev />
        </button>
      </li>
    {/each}
    {#if levels.length < MAX_LEVELS}
      <li><button class="add" disabled={busy} onclick={() => (editing = { open: true, level: null })}><Tile icon="plus" tone="dashed" />{$t('chan_level_add')}</button></li>
    {/if}
  </Box>
  <p class="hint">{$t('chan_levels_hint')}</p>
</section>
{/if}

{#snippet scopeRow(s: ScopeView, n: number)}
  <SectionBadge scope={s} communityId={cid} size={36} noPhoto />
  <span class="row-text">
    <span class="row-name wrap"><span class="nm">{titleOf(s)}</span>{#if s.hidden}<span class="tag hidden-tag"><Icon name="eye-off" size={14} />{$t('chan_scope_hidden_mark')}</span>{/if}{#if s.scheme === 'flat-v1'}<span class="tag">{$t('chan_scope_scheme_flat')}</span>{/if}</span>
    {#if s.note}<span class="row-sub">{s.note}</span>{/if}
    <span class="row-sub dim">
      {#if s.scope_id === EVERYONE}{$t('chan_scope_everyone_line')}
      {:else if n}{$t(countKey('chan_scope_given', n, $locale), { n: String(n) })}
      {:else}{$t('chan_scope_given_none')}{/if}
      {#if s.history === 'from_join'} · {$t('chan_history_from_join')}{/if}
    </span>
  </span>
{/snippet}

{#if part !== 'levels'}
<section class="block" id="chan-sections" aria-label={$t('chan_sections')}>
  <Box tag="ul">
    {#each scopes as s (s.scope_id)}
      {@const n = grantingCount(lv.levels, s.scope_id)}
      <li class="row sec">
        <!-- The owner or an admin sets a section up (§14.7): its title, its look, hidden or not, its place. -->
        {#if canSetUpSections(detail)}
          <button type="button" class="row-main as-button" onclick={() => (scopeDialog = { open: true, id: s.scope_id })}>{@render scopeRow(s, n)}<Chev /></button>
        {:else}
          <span class="row-main">{@render scopeRow(s, n)}</span>
        {/if}
        {#if s.scope_id !== EVERYONE && canSetUpSections(detail)}
          <button class="icon danger" disabled={busy} onclick={() => removeScope(s)} title={$t('chan_scope_delete')} aria-label={$t('chan_scope_delete')}><Icon name="trash-2" size={18} /></button>
        {:else if canSetUpSections(detail)}
          <span class="icon-space"></span>
        {/if}
      </li>
    {/each}
    {#if scopes.length < MAX_SCOPES}
      <li><button class="add" disabled={busy} onclick={() => (scopeDialog = { open: true, id: null })}><Tile icon="folder-plus" tone="dashed" />{$t('chan_scope_add')}</button></li>
    {/if}
  </Box>
  {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
</section>
{/if}

<LevelDialog bind:open={editing.open} {detail} level={editing.level} />
<ScopeDialog bind:open={scopeDialog.open} {detail} scopeId={scopeDialog.id} />

<style>
  .block { display: flex; flex-direction: column; gap: var(--sp-3); }
  /* A row of the box (the hairline between two is the box's). */
  .row {
    display: flex; align-items: center; gap: var(--sp-3); min-height: 52px; padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-3);
    border-radius: var(--radius-md); min-width: 0; color: var(--text-2);
  }
  .row.sec { padding: 0 var(--sp-1) 0 0; gap: 2px; }
  .row-main { flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--sp-3); padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-3); color: inherit; border-radius: var(--radius-md); }
  .row-name.wrap { flex-wrap: wrap; row-gap: var(--sp-1); }
  .nm { min-width: 0; }
  .as-button { width: 100%; border: none; background: none; font: inherit; text-align: left; cursor: pointer; transition: background var(--dur-fast) var(--ease); }
  .as-button:hover { background: var(--surface-row-hover); }
  .row-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .row-name { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); min-width: 0; overflow-wrap: anywhere; }
  .row-sub { display: flex; align-items: flex-start; gap: var(--sp-1); font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; min-width: 0; }
  .row-sub.dim { color: var(--text-3); }
  .row-sub :global(svg) { flex-shrink: 0; margin-top: 1px; color: var(--text-3); }
  .tag {
    flex-shrink: 0; display: inline-flex; align-items: center; gap: var(--sp-1); height: 20px; padding: 0 var(--sp-2); border-radius: var(--radius-pill);
    font-size: var(--fs-2xs); font-weight: var(--fw-semibold); background: var(--surface-3); color: var(--text-2);
  }
  .hidden-tag { background: var(--danger-bg); color: var(--danger-text); }
  .members { display: inline-flex; align-items: center; gap: var(--sp-1); font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-2); font-variant-numeric: tabular-nums; flex-shrink: 0; }
  .add {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 52px; padding: var(--sp-2) var(--sp-3); border: none; background: none;
    border-radius: var(--radius-md); color: var(--accent-text); font: inherit; font-size: var(--fs-base); font-weight: var(--fw-bold); text-align: left; cursor: pointer;
  }
  .add:hover:not(:disabled) { background: var(--surface-row-hover); }
  .icon { flex: none; border: none; background: none; color: var(--text-2); cursor: pointer; display: grid; place-items: center; width: 32px; height: 32px; padding: 0; border-radius: var(--radius-sm); }
  .icon:hover { background: var(--surface-3); }
  .icon.danger { color: var(--danger-text); }
  .icon-space { flex: none; width: 32px; }
  .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-3); line-height: 1.45; }
  @media (pointer: coarse) {
    .row, .add { min-height: 56px; }
    .icon, .icon-space { width: 40px; height: 40px; }
  }
</style>

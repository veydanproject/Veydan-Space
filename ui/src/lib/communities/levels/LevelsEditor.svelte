<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The levels and sections of a channel, for its managers (communities-wire
  §14.1, spec §5): each level with its name, the sections it opens, its
  rights and how many have it; each section with the levels that open it.
  A level is made and changed in its dialog; a section is added by an
  admin and deleted by the owner, after a question that names how many
  levels lose it. Everyone's section and the base level always stay.
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

  interface Props {
    detail: ChannelDetail;
  }
  let { detail }: Props = $props();

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

<section class="block" aria-label={$t('chan_levels')}>
  <div class="label-row">
    <span class="label">{$t('chan_levels')} · {levels.length}</span>
    {#if levels.length < MAX_LEVELS}
      <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => (editing = { open: true, level: null })}><Icon name="plus" size={13} />{$t('chan_level_add')}</button>
    {/if}
  </div>
  <ul class="list divided">
    {#each levels as l (l.id)}
      <li>
        <button class="row as-button" onclick={() => (editing = { open: true, level: l })}>
          <span class="row-icon" class:base={l.id === BASE_LEVEL}><Icon name={l.id === BASE_LEVEL ? 'users' : 'layers'} size={15} /></span>
          <span class="row-text">
            <span class="row-name">{l.name}{#if l.id === BASE_LEVEL}<span class="tag">{$t('chan_level_base')}</span>{/if}</span>
            <span class="row-sub"><Icon name="folder" size={11} />{opens(l)}</span>
            {#if rights(l).length}<span class="row-sub"><Icon name="pencil" size={11} />{rights(l).join(' · ')}</span>{/if}
          </span>
          <span class="members" title={$t(countKey('chan_level_members', l.members, $locale), { n: fmt(l.members) })}><Icon name="user" size={12} />{fmt(l.members)}</span>
          <Icon name="chevron-right" size={14} />
        </button>
      </li>
    {/each}
  </ul>
  <p class="hint">{$t('chan_levels_hint')}</p>
</section>

{#snippet scopeRow(s: ScopeView, n: number)}
  <SectionBadge scope={s} communityId={cid} size={32} />
  <span class="row-text">
    <span class="row-name wrap"><span class="nm">{titleOf(s)}</span>{#if s.hidden}<span class="tag hidden-tag"><Icon name="eye-off" size={11} />{$t('chan_scope_hidden_mark')}</span>{/if}{#if s.scheme === 'flat-v1'}<span class="tag">{$t('chan_scope_scheme_flat')}</span>{/if}</span>
    {#if s.note}<span class="row-sub">{s.note}</span>{/if}
    <span class="row-sub">
      {#if s.scope_id === EVERYONE}{$t('chan_scope_everyone_line')}
      {:else if n}{$t(countKey('chan_scope_given', n, $locale), { n: String(n) })}
      {:else}{$t('chan_scope_given_none')}{/if}
      {#if s.history === 'from_join'} · {$t('chan_history_from_join')}{/if}
    </span>
  </span>
{/snippet}

<section class="block" id="chan-sections" aria-label={$t('chan_sections')}>
  <div class="label-row">
    <span class="label">{$t('chan_sections')} · {scopes.length}</span>
    {#if scopes.length < MAX_SCOPES}
      <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => (scopeDialog = { open: true, id: null })}><Icon name="folder-plus" size={13} />{$t('chan_scope_add')}</button>
    {/if}
  </div>
  <ul class="list divided">
    {#each scopes as s (s.scope_id)}
      {@const n = grantingCount(lv.levels, s.scope_id)}
      <li class="row">
        <!-- The owner or an admin sets a section up (§14.7): its title, its look, hidden or not, its place. -->
        {#if canSetUpSections(detail)}
          <button type="button" class="row-main as-button" onclick={() => (scopeDialog = { open: true, id: s.scope_id })}>{@render scopeRow(s, n)}<Icon name="chevron-right" size={14} /></button>
        {:else}
          <span class="row-main">{@render scopeRow(s, n)}</span>
        {/if}
        {#if s.scope_id !== EVERYONE && canSetUpSections(detail)}
          <button class="icon danger" disabled={busy} onclick={() => removeScope(s)} title={$t('chan_scope_delete')} aria-label={$t('chan_scope_delete')}><Icon name="trash-2" size={14} /></button>
        {/if}
      </li>
    {/each}
  </ul>
  {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
</section>

<LevelDialog bind:open={editing.open} {detail} level={editing.level} />
<ScopeDialog bind:open={scopeDialog.open} {detail} scopeId={scopeDialog.id} />

<style>
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .label { font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.6px; color: var(--text-3); font-weight: var(--fw-bold); }
  .label-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); flex-wrap: wrap; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; --rows-gap: 2px; }
  .row { display: flex; align-items: center; gap: var(--sp-3); padding: 6px 8px; border-radius: var(--radius-sm); min-width: 0; color: var(--text-2); }
  .row-main { flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--sp-3); color: inherit; border-radius: var(--radius-sm); }
  li.row:has(> .row-main) { padding: 0 4px 0 0; }
  .row-main { padding: 6px 8px; }
  .row-name.wrap { flex-wrap: wrap; row-gap: 2px; }
  .nm { min-width: 0; }
  .hidden-tag { display: inline-flex; align-items: center; gap: 3px; color: var(--danger-text); border-color: var(--danger-border); }
  .as-button { width: 100%; border: none; background: none; font: inherit; text-align: left; cursor: pointer; }
  .as-button:hover { background: var(--surface-row-hover); }
  .row-icon {
    width: 32px; height: 32px; flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center;
    border-radius: var(--radius-sm); background: var(--surface-3); color: var(--text-2);
  }
  .row-icon.base { background: var(--accent-tint); color: var(--accent-text-2); }
  .row-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .row-name { display: flex; align-items: center; gap: 6px; font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); min-width: 0; overflow-wrap: anywhere; }
  .row-sub { display: flex; align-items: center; gap: 4px; font-size: var(--fs-2xs); color: var(--text-3); line-height: 1.4; min-width: 0; }
  .row-sub :global(svg) { flex-shrink: 0; }
  .tag { flex-shrink: 0; font-size: var(--fs-2xs); font-weight: var(--fw-bold); padding: 0 6px; border-radius: var(--radius-pill); border: 1px solid var(--border-strong); color: var(--text-2); }
  .members { display: inline-flex; align-items: center; gap: 3px; font-size: var(--fs-xs); color: var(--text-2); font-variant-numeric: tabular-nums; flex-shrink: 0; }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .icon:hover { background: var(--surface-3); }
  .icon.danger { color: var(--danger-text); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  @media (pointer: coarse) {
    .row { padding: 9px 8px; }
    .icon { padding: 10px; }
  }
</style>

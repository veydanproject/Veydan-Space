<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A level made or changed by a manager of the channel (communities-wire
  §14.1, `level_set`): its name, which no build gives (the owner names
  them), the sections it opens and its rights. The base level of everyone
  who entered is renamed here, keeps the section of everyone and is never
  deleted; another level is deleted from here too. A new one tells what a
  level is and what it opens behind a «?» beside each term (TermHint).
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { countKey, locale, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { confirmStore } from '$lib/messenger/seams/shared';
  import { channelStore } from '../channelStore.svelte';
  import { channelError } from '../errors';
  import { isModerator } from '../permissions';
  import type { ChannelDetail } from '../api';
  import { BASE_LEVEL, EVERYONE, levelsOf } from './sections';
  import { LEVEL_NAME_MAX, RIGHTS, blankLevel, checkLevel, formOf, normalizeLevel, type LevelProblem } from './editor';
  import LevelPeople from './LevelPeople.svelte';
  import TermHint from '$lib/core/ui/TermHint.svelte';
  import type { LevelInput, LevelView } from './types';

  interface Props {
    open: boolean;
    detail: ChannelDetail;
    /** The level changed; `null`: a new one. */
    level: LevelView | null;
  }
  let { open = $bindable(), detail, level }: Props = $props();
  const uid = $props.id();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const cid = $derived(detail.channel.id);
  const lv = $derived(levelsOf(detail));
  const base = $derived(level?.id === BASE_LEVEL);
  /** The sections a level may open: everyone's is the base level's alone. */
  const choices = $derived(lv.scopes.filter((s) => s.scope_id !== EVERYONE));

  let form = $state<LevelInput>(blankLevel());
  let tried = $state(false);
  let busy = $state(false);
  let error = $state('');

  // Each opening starts from the level (or a blank one).
  let openedFor: LevelView | null | undefined;
  $effect(() => {
    if (!open) { openedFor = undefined; return; }
    if (openedFor === level) return;
    openedFor = level;
    form = level ? formOf(level) : blankLevel();
    tried = false; error = ''; busy = false;
  });

  const problem = $derived<LevelProblem | null>(checkLevel(form, lv.levels));
  const PROBLEM: Record<LevelProblem, string> = {
    name_empty: 'chan_level_err_name', name_long: 'chan_level_err_long', name_taken: 'chan_level_err_taken', level_limit: 'chan_err_level_limit',
  };

  function toggleScope(id: string) {
    form.scopes = form.scopes.includes(id) ? form.scopes.filter((s) => s !== id) : [...form.scopes, id];
  }

  async function save() {
    tried = true;
    if (problem) return;
    busy = true; error = '';
    try {
      await channelStore.setLevel(cid, normalizeLevel(form));
      open = false;
    } catch (e) {
      error = channelError(e, tr);
    } finally {
      busy = false;
    }
  }

  async function remove() {
    if (!level || base) return;
    const n = level.members;
    const text = $t(countKey('chan_level_delete_confirm', n, $locale), { name: level.name, n: String(n) });
    if (!(await confirmStore.ask(text, $t('chan_level_delete'), true))) return;
    busy = true; error = '';
    try {
      await channelStore.delLevel(cid, level.id);
      open = false;
    } catch (e) {
      error = channelError(e, tr);
    } finally {
      busy = false;
    }
  }
</script>

<!-- A new one: beside its title, the «?» of what it is. -->
{#snippet newAside()}<TermHint term={$t('chan_level_new')}>{$t('chan_levels_hint')}</TermHint>{/snippet}

<Dialog bind:open title={level ? $t('chan_level_edit', { name: level.name }) : $t('chan_level_new')} titleAside={level ? undefined : newAside} width="min(640px, calc(100vw - 24px))">
  <div class="body">
    {#if base}<p class="hint base"><Icon name="users" size={13} />{$t('chan_level_base_hint')}</p>{/if}
    <label class="field-wrap">
      <span class="label">{$t('chan_level_name')}</span>
      <input class="field" type="text" bind:value={form.name} maxlength={LEVEL_NAME_MAX + 10} placeholder={$t('chan_level_name_ph')}
        aria-invalid={tried && !!problem} disabled={busy} />
      <span class="count" class:over={[...form.name.trim()].length > LEVEL_NAME_MAX}>{[...form.name.trim()].length}/{LEVEL_NAME_MAX}</span>
    </label>
    {#if tried && problem}<div class="error-msg" role="alert">{$t(PROBLEM[problem] as 'chan_level_err_name')}</div>{/if}

    <!-- The sections and the rights side by side (one under the other in a narrow window). -->
    <div class="cols">
    <!-- Named by its words alone: the «?» right after them is not in a legend, or the group would be read «Открывает разделы Открывает разделы». -->
    <fieldset class="group" aria-labelledby="{uid}-scopes">
      <div class="term"><span class="label" id="{uid}-scopes">{$t('chan_level_scopes')}</span>{#if !level}<TermHint term={$t('chan_level_scopes')}>{$t('chan_levels_explain')}</TermHint>{/if}</div>
      {#if base || choices.length}
        <div class="box">
          {#if base}
            <label class="check"><input type="checkbox" checked disabled /><span>{lv.scopes.find((s) => s.scope_id === EVERYONE)?.title || $t('chan_scope_everyone')}</span></label>
          {/if}
          {#each choices as s (s.scope_id)}
            <label class="check"><input type="checkbox" checked={form.scopes.includes(s.scope_id)} disabled={busy} onchange={() => toggleScope(s.scope_id)} /><span>{s.title}</span></label>
          {/each}
        </div>
      {:else}
        <p class="hint">{$t('chan_level_no_sections')}</p>
      {/if}
    </fieldset>

    <fieldset class="group">
      <legend class="label">{$t('chan_level_rights')}</legend>
      <div class="box">
        {#each RIGHTS as r (r)}
          <label class="check"><input type="checkbox" bind:checked={form.can[r]} disabled={busy} />
            <span class="right"><span>{$t(`chan_right_${r}` as 'chan_right_comment')}</span><span class="hint">{$t(`chan_right_${r}_hint` as 'chan_right_comment_hint')}</span></span></label>
        {/each}
      </div>
    </fieldset>
    </div>

    {#if level && !base && isModerator(detail)}<LevelPeople {detail} {level} />{/if}

    {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
  </div>
  {#snippet footer()}
    <!-- The dangerous one at the left of the foot, apart from «Отмена | Сохранить». -->
    {#if level && !base}
      <button type="button" class="btn btn-ghost danger" disabled={busy} onclick={remove}><Icon name="trash-2" size={18} />{$t('chan_level_delete')}</button>
    {/if}
    <button type="button" class="btn btn-ghost" disabled={busy} onclick={() => (open = false)}>{$t('chan_cancel')}</button>
    <button type="button" class="btn btn-primary" disabled={busy || (tried && !!problem)} aria-busy={busy} onclick={save}>
      {#if busy}<span class="spinner light" aria-hidden="true"></span>{/if}{$t('chan_save')}
    </button>
  {/snippet}
</Dialog>

<style>
  /* The spinner of a primary button, on its fill. */
  .spinner.light { border-color: rgba(255, 255, 255, 0.35); border-top-color: #fff; }
  .body { display: flex; flex-direction: column; gap: var(--sp-4); container-type: inline-size; }
  /* «Открывает разделы» beside «Права»: the rights, with their explanations, take the wider column. */
  .cols { display: grid; grid-template-columns: minmax(0, 0.8fr) minmax(0, 1.2fr); gap: var(--sp-3); align-items: start; }
  @container (max-width: 480px) { .cols { grid-template-columns: minmax(0, 1fr); gap: var(--sp-4); } }
  .field-wrap { display: flex; flex-direction: column; gap: var(--sp-2); position: relative; }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-bold); color: var(--text-2); padding: 0; }
  /* The field 44 (48 under a finger), its count inside on the right. */
  .field {
    width: 100%; height: var(--control-h-lg); box-sizing: border-box; font: inherit; font-size: var(--fs-base); color: var(--text); background: var(--surface-3);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: 0 52px 0 var(--sp-3);
  }
  .field:focus { outline: none; border-color: var(--accent-border); }
  .field[aria-invalid='true'] { border-color: var(--danger-border); }
  .count { position: absolute; right: var(--sp-3); bottom: 14px; font-size: var(--fs-xs); color: var(--text-3); font-variant-numeric: tabular-nums; }
  .count.over { color: var(--danger-text); }
  .group { border: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; }
  .group legend, .group .term { margin-bottom: var(--sp-1); }
  /* A legend stands outside the fieldset's gap: its own margin and the height of the «?» give it the same distance as the «?» row, so the two cards line up. */
  .group legend { margin-bottom: var(--sp-3); line-height: 24px; }
  /* The words of a group with its «?»: the row its note opens under. */
  .term { position: relative; display: flex; align-items: center; width: 100%; min-height: 24px; }
  /* Its note keeps the width of the whole body, also from the narrow column of the sections. */
  .group .term :global(.note) { right: auto; width: min(440px, 100cqi); }
  /* The sections and the rights: a card of marks with their explanations. */
  .box { display: flex; flex-direction: column; gap: var(--sp-3); padding: var(--sp-3); border-radius: var(--radius-lg); border: 1px solid var(--border); background: var(--surface-2); }
  .check { display: flex; align-items: flex-start; gap: var(--sp-3); font-size: var(--fs-base); font-weight: var(--fw-semibold); color: var(--text); cursor: pointer; user-select: none; }
  .check input { margin-top: 2px; flex-shrink: 0; }
  .right { display: flex; flex-direction: column; gap: 2px; }
  .right .hint { font-weight: var(--fw-normal); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  .hint.base { display: flex; gap: var(--sp-2); align-items: flex-start; padding: var(--sp-2) var(--sp-3); border-radius: var(--radius-md); background: var(--surface-2); }
  .hint.base :global(svg) { flex-shrink: 0; margin-top: 2px; }
  /* «Удалить уровень»: a ghost in the danger colour, 40 high, pushed to the left of the foot. */
  .danger { color: var(--danger-text); gap: var(--sp-2); margin-right: auto; }
  @media (pointer: coarse) {
    .field { font-size: var(--fs-md); height: 48px; }
    .count { bottom: 16px; }
    .check { padding: var(--sp-1) 0; }
    .term { min-height: 32px; }
    .group legend { line-height: 32px; }
  }
</style>

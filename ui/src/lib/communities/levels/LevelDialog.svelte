<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A level made or changed by a manager of the channel (communities-wire
  §14.1, `level_set`): its name, which no build gives (the owner names
  them), the sections it opens and its rights. The base level of everyone
  who entered is renamed here, keeps the section of everyone and is never
  deleted; another level is deleted from here too.
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
  import type { LevelInput, LevelView } from './types';

  interface Props {
    open: boolean;
    detail: ChannelDetail;
    /** The level changed; `null`: a new one. */
    level: LevelView | null;
  }
  let { open = $bindable(), detail, level }: Props = $props();

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

<Dialog bind:open title={level ? $t('chan_level_edit', { name: level.name }) : $t('chan_level_new')} width="min(460px, calc(100vw - 24px))">
  <div class="body">
    {#if base}<p class="hint base"><Icon name="users" size={13} />{$t('chan_level_base_hint')}</p>{/if}
    <label class="field-wrap">
      <span class="label">{$t('chan_level_name')}</span>
      <input class="field" type="text" bind:value={form.name} maxlength={LEVEL_NAME_MAX + 10} placeholder={$t('chan_level_name_ph')}
        aria-invalid={tried && !!problem} disabled={busy} />
      <span class="count" class:over={[...form.name.trim()].length > LEVEL_NAME_MAX}>{[...form.name.trim()].length}/{LEVEL_NAME_MAX}</span>
    </label>
    {#if tried && problem}<div class="error-msg" role="alert">{$t(PROBLEM[problem] as 'chan_level_err_name')}</div>{/if}

    <fieldset class="group">
      <legend class="label">{$t('chan_level_scopes')}</legend>
      {#if base}
        <label class="check"><input type="checkbox" checked disabled /><span>{lv.scopes.find((s) => s.scope_id === EVERYONE)?.title || $t('chan_scope_everyone')}</span></label>
      {/if}
      {#each choices as s (s.scope_id)}
        <label class="check"><input type="checkbox" checked={form.scopes.includes(s.scope_id)} disabled={busy} onchange={() => toggleScope(s.scope_id)} /><span>{s.title}</span></label>
      {:else}
        {#if !base}<p class="hint">{$t('chan_level_no_sections')}</p>{/if}
      {/each}
    </fieldset>

    <fieldset class="group">
      <legend class="label">{$t('chan_level_rights')}</legend>
      {#each RIGHTS as r (r)}
        <label class="check"><input type="checkbox" bind:checked={form.can[r]} disabled={busy} />
          <span class="right"><span>{$t(`chan_right_${r}` as 'chan_right_comment')}</span><span class="hint">{$t(`chan_right_${r}_hint` as 'chan_right_comment_hint')}</span></span></label>
      {/each}
    </fieldset>

    {#if level && !base && isModerator(detail)}<LevelPeople {detail} {level} />{/if}

    {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
    {#if level && !base}
      <button type="button" class="btn btn-ghost btn-sm danger" disabled={busy} onclick={remove}><Icon name="trash-2" size={13} />{$t('chan_level_delete')}</button>
    {/if}
  </div>
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" disabled={busy} onclick={() => (open = false)}>{$t('chan_cancel')}</button>
    <button type="button" class="btn btn-primary" disabled={busy || (tried && !!problem)} aria-busy={busy} onclick={save}>
      {#if busy}<span class="spinner light" aria-hidden="true"></span>{/if}{$t('chan_save')}
    </button>
  {/snippet}
</Dialog>

<style>
  /* The spinner of a primary button, on its fill. */
  .spinner.light { border-color: rgba(255, 255, 255, 0.35); border-top-color: #fff; }
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .field-wrap { display: flex; flex-direction: column; gap: 4px; position: relative; }
  .label { font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-2); padding: 0; }
  .field {
    width: 100%; font: inherit; font-size: var(--fs-sm); color: var(--text); background: var(--surface-2);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: 8px 52px 8px 10px;
  }
  .field:focus { outline: none; border-color: var(--accent-border); }
  .field[aria-invalid='true'] { border-color: var(--danger-border); }
  .count { position: absolute; right: 10px; bottom: 9px; font-size: var(--fs-2xs); color: var(--text-3); font-variant-numeric: tabular-nums; }
  .count.over { color: var(--danger-text); }
  .group { border: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .group legend { margin-bottom: 4px; }
  .check { display: flex; align-items: flex-start; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--text-body); cursor: pointer; user-select: none; }
  .check input { margin-top: 3px; flex-shrink: 0; }
  .right { display: flex; flex-direction: column; gap: 1px; }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  .hint.base { display: flex; gap: 6px; align-items: flex-start; padding: var(--sp-2) var(--sp-3); border-radius: var(--radius-md); background: var(--surface-2); }
  .hint.base :global(svg) { flex-shrink: 0; margin-top: 2px; }
  .danger { color: var(--danger-text); align-self: flex-start; }
  @media (pointer: coarse) {
    .field { font-size: var(--fs-md); }
    .check { padding: 4px 0; }
  }
</style>

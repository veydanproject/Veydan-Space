<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/browser/api';
  import type { CaptureRule } from '$lib/browser/types';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { directory } from '$lib/core/directory';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';

  interface Row { domain: string; folder_id: string; tags: string; template_id: string }

  let rows = $state<Row[]>([]);
  let loaded = $state(false);
  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  // The folders and templates of the notes come through the catalog of
  // entities: the clipper writes into notes, but this module does not read them.
  const folders = $derived(directory.list('note_folder'));
  const templates = $derived(directory.list('note_template'));

  onMount(async () => {
    await Promise.all([
      directory.get('note_folder')?.ensureLoaded().catch(() => {}),
      directory.get('note_template')?.ensureLoaded().catch(() => {}),
    ]);
    try {
      rows = (await api.capture.rulesGet()).map(toRow);
    } catch {}
    loaded = true;
  });

  function toRow(r: CaptureRule): Row {
    return { domain: r.domain, folder_id: r.folder_id ?? '', tags: r.tags.join(', '), template_id: r.template_id ?? '' };
  }

  function toRule(r: Row): CaptureRule {
    return {
      domain: r.domain.trim(),
      folder_id: r.folder_id || null,
      tags: r.tags.split(',').map((s) => s.trim()).filter(Boolean),
      template_id: r.template_id || null,
    };
  }

  /** Debounced persist; empty domains are kept locally until filled */
  function scheduleSave() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(async () => {
      try {
        await api.capture.rulesSet(rows.map(toRule).filter((r) => r.domain));
      } catch {}
    }, 400);
  }

  function addRow() {
    rows = [...rows, { domain: '', folder_id: '', tags: '', template_id: '' }];
  }

  function removeRow(i: number) {
    rows = rows.filter((_, idx) => idx !== i);
    scheduleSave();
  }
</script>

<!-- One card of rules under column captions, each rule a row of four fields and its delete; the add at
     the foot. No rules: the empty line and the add in the middle. A narrow place: the domain on a line of
     its own, the rest under it. -->
{#snippet add()}
  <button type="button" class="btn btn-ghost add" onclick={addRow}>
    <Icon name="plus" size={18} /> {$t('settings_capture_add')}
  </button>
{/snippet}

<div class="card rules">
  {#if loaded && rows.length === 0}
    <div class="empty">
      <p>{$t('settings_capture_empty')}</p>
      {@render add()}
    </div>
  {:else}
    {#if rows.length > 0}
      <div class="head" aria-hidden="true">
        <span>{$t('settings_capture_domain')}</span>
        <span>{$t('settings_capture_folder')}</span>
        <span>{$t('settings_capture_tags')}</span>
        <span>{$t('note_template_label')}</span>
        <span></span>
      </div>
    {/if}
    {#each rows as row, i (i)}
      <div class="rule-row">
        <input
          class="rule-input domain"
          type="text"
          placeholder={$t('settings_capture_domain')}
          aria-label={$t('settings_capture_domain')}
          bind:value={row.domain}
          oninput={scheduleSave}
        />
        <div class="rule-pick folder">
          <CustomSelect
            options={[{ value: '', label: $t('settings_capture_no_folder') }, ...folders.map((f) => ({ value: f.id, label: f.name }))]}
            bind:value={row.folder_id}
            onchange={scheduleSave}
            ariaLabel={$t('settings_capture_folder')}
            title={$t('settings_capture_folder')}
          />
        </div>
        <input
          class="rule-input tags"
          type="text"
          placeholder={$t('settings_capture_tags')}
          aria-label={$t('settings_capture_tags')}
          bind:value={row.tags}
          oninput={scheduleSave}
        />
        <div class="rule-pick template">
          <CustomSelect
            options={[{ value: '', label: $t('note_template_none') }, ...templates.map((n) => ({ value: n.id, label: n.name }))]}
            bind:value={row.template_id}
            onchange={scheduleSave}
            ariaLabel={$t('note_template_label')}
            title={$t('note_template_label')}
          />
        </div>
        <button type="button" class="btn btn-ghost del" onclick={() => removeRow(i)} aria-label={$t('common_delete')} title={$t('common_delete')}>
          <Icon name="trash-2" size={18} />
        </button>
      </div>
    {/each}
    <div class="foot">{@render add()}</div>
  {/if}
</div>

<style>
  .rules { padding: var(--sp-3) var(--sp-4) var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-2); container-type: inline-size; }
  /* A list is not narrower than 160, a text field than 180. */
  .head, .rule-row {
    display: grid;
    grid-template-columns: minmax(0, 1.2fr) minmax(160px, 1fr) minmax(180px, 1.2fr) minmax(160px, 1fr) var(--control-h);
    gap: var(--sp-2);
    align-items: center;
  }
  .head { padding: 0 var(--sp-1); font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-dim); }
  .rule-input {
    min-width: 0;
    height: var(--control-h);
    padding: 0 var(--sp-3);
    font-size: var(--fs-sm);
    border: 1px solid var(--border);
    border-radius: var(--radius-field);
    background: var(--surface-3);
    color: var(--text);
  }
  .rule-input:focus { border-color: var(--accent-border); outline: none; }
  .rule-pick { min-width: 0; }
  .del { width: var(--control-h); height: var(--control-h); padding: 0; justify-content: center; color: var(--text-2); }
  .foot { margin-top: var(--sp-1); padding-top: var(--sp-2); border-top: 1px solid var(--divider); }
  .add { height: var(--control-h); display: inline-flex; align-items: center; gap: var(--sp-2); }
  .empty { display: flex; flex-direction: column; align-items: center; gap: var(--sp-3); padding: var(--sp-6) 0 var(--sp-3); }
  .empty p { margin: 0; font-size: var(--fs-base); color: var(--text-2); }

  /* A narrow place (the four in one line would leave the domain under 160): the domain over the
     whole width with the delete on its right, then folder, tags and template, wrapping as they fit. */
  @container (max-width: 760px) {
    .head { display: none; }
    .rule-row { display: flex; flex-wrap: wrap; padding: var(--sp-3) 0; border-bottom: 1px solid var(--divider); }
    .domain { flex: 1 1 calc(100% - var(--control-h) - var(--sp-2)); order: 0; }
    .del { flex: 0 0 auto; order: 1; }
    .folder { flex: 1 1 160px; min-width: 160px; order: 2; }
    .tags { flex: 1 1 180px; min-width: 180px; order: 3; }
    .template { flex: 1 1 160px; min-width: 160px; order: 4; }
    .foot { border-top: 0; padding-top: 0; }
  }
</style>

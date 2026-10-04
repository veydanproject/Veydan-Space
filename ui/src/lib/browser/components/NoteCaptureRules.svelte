<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/browser/api';
  import type { CaptureRule } from '$lib/browser/types';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { directory } from '$lib/core/directory';

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

<div class="rules">
  {#if loaded && rows.length === 0}
    <p class="empty">{$t('settings_capture_empty')}</p>
  {/if}
  {#each rows as row, i (i)}
    <div class="rule-row">
      <input
        class="rule-input"
        type="text"
        placeholder={$t('settings_capture_domain')}
        bind:value={row.domain}
        oninput={scheduleSave}
      />
      <select class="rule-input" bind:value={row.folder_id} onchange={scheduleSave} title={$t('settings_capture_folder')}>
        <option value="">{$t('settings_capture_no_folder')}</option>
        {#each folders as f (f.id)}
          <option value={f.id}>{f.name}</option>
        {/each}
      </select>
      <input
        class="rule-input"
        type="text"
        placeholder={$t('settings_capture_tags')}
        bind:value={row.tags}
        oninput={scheduleSave}
      />
      <select class="rule-input" bind:value={row.template_id} onchange={scheduleSave} title={$t('note_template_label')}>
        <option value="">{$t('note_template_none')}</option>
        {#each templates as n (n.id)}
          <option value={n.id}>{n.name}</option>
        {/each}
      </select>
      <button class="btn btn-ghost btn-sm btn-icon" onclick={() => removeRow(i)} title={$t('notes_tag_delete')}>
        <Icon name="trash-2" size={13} />
      </button>
    </div>
  {/each}
  <button class="btn btn-ghost btn-sm add" onclick={addRow}>
    <Icon name="plus" size={12} /> {$t('settings_capture_add')}
  </button>
</div>

<style>
  .rules { display: flex; flex-direction: column; gap: 6px; }
  .rule-row { display: grid; grid-template-columns: 1.2fr 1fr 1.2fr 1fr auto; gap: 6px; align-items: center; }
  .rule-input {
    min-width: 0;
    padding: 6px 8px;
    font-size: 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-2);
    color: var(--text);
  }
  .add { align-self: flex-start; display: inline-flex; align-items: center; gap: 4px; }
  .empty { margin: 0; font-size: 12px; color: var(--text-2); }
</style>

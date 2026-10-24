<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The notes folder card of the settings page: where the note files live. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { api } from '$lib/notes/api';
  import { formatError } from '$lib/core/utils';

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  let notesDir = $state('');
  let notesDirIsCustom = $state(false);
  let notesDirSaving = $state(false);
  let notesDirError = $state('');

  onMount(async () => {
    try {
      const info = await api.notes.getDir();
      notesDir = info.current;
      notesDirIsCustom = info.is_custom;
    } catch {}
  });

  async function browseNotesDir() {
    if (!isTauri) return;
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({ directory: true, multiple: false, title: $t('notes_dir_pick_title') });
      if (selected && typeof selected === 'string') {
        notesDir = selected;
      }
    } catch {}
  }

  async function saveNotesDir() {
    notesDirSaving = true;
    notesDirError = '';
    try {
      const info = await api.notes.setDir(notesDir.trim() || null);
      notesDir = info.current;
      notesDirIsCustom = info.is_custom;
    } catch (e) {
      notesDirError = formatError(e);
    } finally {
      notesDirSaving = false;
    }
  }

  async function resetNotesDir() {
    notesDirSaving = true;
    notesDirError = '';
    try {
      const info = await api.notes.setDir(null);
      notesDir = info.current;
      notesDirIsCustom = info.is_custom;
    } catch (e) {
      notesDirError = formatError(e);
    } finally {
      notesDirSaving = false;
    }
  }
</script>

<!-- The path and «Browse» in one row; the own folder's note and an error under it; the actions at the foot, the save on the right. -->
<div class="card notes-dir">
  <div class="path-row">
    <input
      class="dir-input"
      type="text"
      bind:value={notesDir}
      placeholder={$t('settings_notes_folder_placeholder')}
      aria-label={$t('settings_notes_section')}
      readonly={!isTauri}
    />
    {#if isTauri}
      <button type="button" class="btn btn-ghost browse" onclick={browseNotesDir}>
        <Icon name="folder-open" size={18} />
        {$t('settings_notes_browse')}
      </button>
    {/if}
  </div>
  {#if notesDirIsCustom}
    <p class="muted own">{$t('settings_notes_custom_active')}</p>
  {/if}
  {#if notesDirError}
    <div class="fail" role="alert"><Icon name="alert-triangle" size={18} /><span>{notesDirError}</span></div>
  {/if}
  <div class="foot">
    {#if notesDirIsCustom}
      <button type="button" class="btn btn-ghost" disabled={notesDirSaving} onclick={resetNotesDir}>
        {$t('settings_notes_reset')}
      </button>
    {/if}
    <button type="button" class="btn btn-primary" disabled={notesDirSaving} onclick={saveNotesDir}>
      {notesDirSaving ? $t('settings_notes_saving') : $t('settings_notes_save')}
    </button>
  </div>
</div>

<style>
  .notes-dir { padding: var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-3); }
  .path-row { display: flex; align-items: center; gap: var(--sp-2); }
  .browse { height: var(--control-h-lg); flex-shrink: 0; }
  /* Two classes: the page's own `.muted` size must not win by the order of the sheets. */
  .muted.own { margin: 0; font-size: var(--fs-sm); }
  .fail {
    display: flex; align-items: flex-start; gap: var(--sp-3); padding: var(--sp-3) var(--sp-4);
    border-radius: var(--radius-md); background: var(--danger-bg); color: var(--text); font-size: var(--fs-base); line-height: 1.5;
    overflow-wrap: anywhere;
  }
  .fail :global(svg) { flex-shrink: 0; margin-top: 2px; color: var(--danger-text); }
  .foot { display: flex; justify-content: flex-end; flex-wrap: wrap; gap: var(--sp-2); padding-top: var(--sp-3); border-top: 1px solid var(--divider); }
</style>

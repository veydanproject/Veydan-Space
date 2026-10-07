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

<div class="dir-row">
  <input
    class="dir-input"
    type="text"
    bind:value={notesDir}
    placeholder={$t('settings_notes_folder_placeholder')}
    readonly={!isTauri}
  />
  {#if isTauri}
    <button class="btn btn-ghost btn-sm btn-icon" onclick={browseNotesDir} title={$t('settings_notes_browse')}>
      <Icon name="folder-open" size={14} />
    </button>
  {/if}
</div>
<div class="btn-row">
  <button class="btn btn-primary btn-sm" disabled={notesDirSaving} onclick={saveNotesDir}>
    {notesDirSaving ? $t('settings_notes_saving') : $t('settings_notes_save')}
  </button>
  {#if notesDirIsCustom}
    <button class="btn btn-ghost btn-sm" disabled={notesDirSaving} onclick={resetNotesDir}>
      {$t('settings_notes_reset')}
    </button>
  {/if}
</div>
{#if notesDirIsCustom}
  <p class="muted small">{$t('settings_notes_custom_active')}</p>
{/if}
{#if notesDirError}
  <div class="error-msg">{notesDirError}</div>
{/if}

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The notes tab of a browser profile's panel (an entity view of scope `profile`): the profile's notes, and the notes drawer behind them. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t, locale, countKey } from '$lib/core/i18n';
  import { relTime as fmtRelTime } from '$lib/core/utils';
  import NotesPanel from '$lib/notes/components/NotesPanel.svelte';
  import { notesStore } from '$lib/notes/store/notes.svelte';

  let { id, workspaceId = '' }: { id: string; workspaceId?: string } = $props();

  let notesOpen = $state(false);
  let noteToOpen = $state<string | null>(null);

  $effect(() => {
    if (!notesOpen) noteToOpen = null;
  });

  onMount(() => {
    void notesStore.ensureLoaded();
  });

  const profileNotes = $derived(
    notesStore.list
      .filter((n) => n.bindings.includes(`profile:${id}`) && !n.archived)
      .sort((a, b) => b.updated_at.localeCompare(a.updated_at))
  );

  const relTime = (iso: string): string => fmtRelTime(iso, $locale);
</script>

<div class="notes-inline">
  <div class="notes-inline-header">
    <span class="notes-count">{$t(countKey('panel_notes_count', profileNotes.length, $locale), { n: String(profileNotes.length) })}</span>
    <button class="btn-open-notes" onclick={() => (notesOpen = true)}>
      <Icon name="external-link" size={12} /> {$t('panel_notes_open')}
    </button>
  </div>
  {#if notesStore.loading}
    <div class="notes-empty">{$t('loading')}</div>
  {:else if profileNotes.length === 0}
    <div class="notes-empty">{$t('panel_notes_empty')}</div>
  {:else}
    <ul class="notes-list-inline">
      {#each profileNotes as note (note.id)}
        <li>
          <button class="note-card-inline" onclick={() => { noteToOpen = note.id; notesOpen = true; }}>
            <div class="note-card-top">
              <span class="note-title-inline">{note.title || $t('notes_untitled')}</span>
              <span class="note-badge">{note.format.toUpperCase()}</span>
            </div>
            {#if note.preview}
              <span class="note-preview-inline">{note.preview}</span>
            {/if}
            <div class="note-card-bottom">
              <span class="note-time-inline">{relTime(note.updated_at)}</span>
              <span class="note-flags">
                {#if note.pinned}<span class="note-flag">📌</span>{/if}
                {#if note.has_draft}<span class="note-flag draft-flag">{$t('panel_notes_draft')}</span>{/if}
              </span>
            </div>
            {#if note.tags.length > 0}
              <div class="note-tags-inline">
                {#each note.tags.slice(0, 3) as tag}
                  <span class="note-tag-chip tinted" style:--chip={tag.color}>{tag.name}</span>
                {/each}
                {#if note.tags.length > 3}<span class="note-tag-more">+{note.tags.length - 3}</span>{/if}
              </div>
            {/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<NotesPanel
  bind:open={notesOpen}
  context="profile"
  contextId={id}
  workspaceId={workspaceId}
  openNoteId={noteToOpen}
/>

<style>
  .notes-inline {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    padding: var(--sp-3);
  }
  .notes-inline-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--sp-1);
  }
  .notes-count {
    font-size: var(--fs-sm);
    color: var(--text-2);
  }
  .notes-empty {
    font-size: var(--fs-base);
    color: var(--text-2);
    padding: var(--sp-2) 0;
  }
  .notes-list-inline {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }
  .note-card-inline {
    width: 100%;
    text-align: left;
    background: var(--bg-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: var(--sp-2) 0.65rem;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    transition: background 0.15s;
  }
  .note-card-inline:hover { background: var(--surface-2); }
  .note-card-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.4rem;
  }
  .note-title-inline {
    font-size: var(--fs-sm);
    font-weight: 500;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
  }
  .note-badge {
    font-size: var(--fs-2xs);
    font-weight: 600;
    color: var(--text-2);
    background: var(--surface-2);
    border-radius: 3px;
    padding: 0.1rem 0.3rem;
    flex-shrink: 0;
  }
  .note-preview-inline {
    font-size: var(--fs-xs);
    color: var(--text-2);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    line-height: 1.4;
  }
  .note-card-bottom {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.4rem;
  }
  .note-time-inline {
    font-size: var(--fs-2xs);
    color: var(--text-3, var(--text-2));
  }
  .note-flags {
    display: flex;
    gap: var(--sp-1);
    align-items: center;
  }
  .note-flag {
    font-size: var(--fs-2xs);
  }
  .draft-flag {
    background: var(--warn-text);
    color: #fff;
    border-radius: 3px;
    padding: 0.05rem 0.25rem;
    font-size: var(--fs-2xs);
    font-weight: 600;
  }
  .note-tags-inline {
    display: flex;
    flex-wrap: wrap;
    gap: 0.2rem;
    margin-top: 0.1rem;
  }
  .note-tag-chip {
    font-size: var(--fs-2xs);
    border-radius: 3px;
    padding: 0.05rem 0.3rem;
    font-weight: 500;
  }
  .note-tag-more {
    font-size: var(--fs-2xs);
    color: var(--text-2);
  }
  .btn-open-notes {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    background: transparent;
    color: var(--accent);
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    padding: 0.2rem 0.6rem;
    font-size: var(--fs-sm);
    cursor: pointer;
    transition: background 0.15s;
  }

  .btn-open-notes:hover { background: color-mix(in srgb, var(--accent) 15%, transparent); }
</style>

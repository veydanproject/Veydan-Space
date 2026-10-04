<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Notes bound to or mentioning a Veydan entity, shown inside that entity's panel. -->
<script lang="ts">
  import { api } from '$lib/notes/api';
  import { binding as makeBinding, type EntityKind } from '$lib/core/bindings';
  import type { NoteListItem } from '$lib/notes/types';
  import { notesStore } from '$lib/notes/store/notes.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t, locale } from '$lib/core/i18n';
  import { relTime } from '$lib/core/utils';
  import NotesPanel from './NotesPanel.svelte';

  interface Props {
    kind: EntityKind;
    id: string;
    /** Title for a note created from this panel */
    newTitle?: string;
  }

  let { kind, id, newTitle = '' }: Props = $props();

  const binding = $derived(makeBinding(kind, id));
  let notes = $state<NoteListItem[]>([]);
  let panelOpen = $state(false);
  let openId = $state<string | null>(null);

  async function load() {
    try {
      notes = await api.notes.entityNotes(binding);
    } catch {
      notes = [];
    }
  }

  // Reload when the entity changes or the panel closes after edits
  $effect(() => {
    void binding;
    if (!panelOpen) void load();
  });

  function open(noteId: string | null) {
    openId = noteId;
    panelOpen = true;
  }

  async function create() {
    const note = await notesStore.createNote({ title: newTitle || $t('notes_untitled'), bindings: [binding] });
    open(note.id);
  }
</script>

<section class="entity-notes">
  <div class="head">
    <span class="label"><Icon name="file-text" size={12} /> {$t('ctx_entity_notes')} <span class="count">{notes.length}</span></span>
    <button type="button" class="btn btn-ghost btn-sm" onclick={() => open(null)}>
      <Icon name="external-link" size={12} /> {$t('panel_notes_open')}
    </button>
    <button type="button" class="btn btn-ghost btn-sm" onclick={create}>
      <Icon name="plus" size={12} /> {$t('ctx_entity_new_note')}
    </button>
  </div>
  {#if notes.length === 0}
    <div class="empty">{$t('panel_notes_empty')}</div>
  {:else}
    <ul class="list">
      {#each notes as n (n.id)}
        <li>
          <button type="button" class="row" onclick={() => open(n.id)}>
            <span class="title">{n.title || $t('notes_untitled')}</span>
            <span class="time">{relTime(n.updated_at, $locale)}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<NotesPanel bind:open={panelOpen} context={kind} contextId={id} openNoteId={openId} />

<style>
  .entity-notes { display: flex; flex-direction: column; gap: 0.4rem; margin-top: var(--sp-4); padding-top: var(--sp-3); border-top: 1px solid var(--border); }
  .head { display: flex; align-items: center; gap: 0.4rem; }
  .label { display: flex; align-items: center; gap: 0.3rem; flex: 1; font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.06em; color: var(--text-3); font-weight: 600; }
  .count { font-weight: 400; }
  .empty { font-size: var(--fs-xs); color: var(--text-3); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.15rem; }
  .row {
    display: flex; align-items: center; gap: 0.5rem; width: 100%;
    padding: 0.3rem 0.45rem; border: 0; border-radius: 4px;
    background: transparent; color: var(--text); text-align: left; cursor: pointer;
  }
  .row:hover { background: var(--surface-2); }
  .title { flex: 1; font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .time { font-size: var(--fs-2xs); color: var(--text-3); flex-shrink: 0; }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { NoteListItem } from '$lib/notes/types';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { isEntityKind, parseBinding } from '$lib/core/bindings';
  import { entitySummary } from '$lib/notes/context';
  import { noteScope, scopeColor as colorOfScope } from '$lib/notes/list-scope';

  interface Props {
    notes: NoteListItem[];
    activeId: string | null;
    onselect: (id: string) => void;
    /** Offered by the empty state; left out where an empty list is a filter's or a search's result. */
    oncreate?: () => void;
    /** What an empty list says; "No notes yet." by default. */
    emptyText?: string;
    workspaceName?: (id: string) => string;
    workspaceColor?: (id: string) => string;
    profileName?: (id: string) => string;
    folderName?: (id: string) => string;
    folderColor?: (id: string) => string;
  }

  let { notes, activeId, onselect, oncreate, emptyText, workspaceName, workspaceColor, profileName, folderName, folderColor }: Props = $props();

  const sources = $derived({ workspaceName, workspaceColor, profileName, folderName, folderColor });

  // Folders first — original context
  function scopeLabel(n: NoteListItem): string {
    const scope = noteScope(n, sources);
    switch (scope.kind) {
      case 'folder': {
        const name = folderName?.(scope.id) ?? scope.id;
        return scope.more ? `${name} +${scope.more}` : name;
      }
      case 'workspace': return workspaceName?.(scope.id) ?? scope.id;
      case 'profile': return profileName?.(scope.id) ?? scope.id;
      default: return $t('notes_scope_global');
    }
  }

  /** What the scope line already names (see scopeLabel), so the chips do not repeat it. */
  function scopeShown(n: NoteListItem): { folder?: string; binding?: string } {
    const scope = noteScope(n, sources);
    if (scope.kind === 'folder') return { folder: scope.id };
    if (scope.kind === 'workspace' || scope.kind === 'profile') return { binding: scope.binding };
    return {};
  }

  function cardChips(n: NoteListItem) {
    const chips: { label: string; color: string }[] = [];
    const shown = scopeShown(n);
    for (const fid of n.folder_ids) {
      if (fid === shown.folder) continue;
      const label = folderName?.(fid) ?? fid;
      const color = folderColor?.(fid) ?? 'var(--text-2)';
      chips.push({ label, color });
    }
    for (const b of n.bindings) {
      if (b === shown.binding) continue;
      const parsed = parseBinding(b);
      if (!parsed) continue;
      if (isEntityKind(parsed.kind)) {
        const entity = entitySummary(parsed.kind, parsed.value);
        if (entity) chips.push({ label: entity.name, color: entity.color });
      } else if (parsed.kind === 'domain') {
        chips.push({ label: parsed.value, color: 'var(--text-2)' });
      }
    }
    for (const t of n.tags) chips.push({ label: t.name, color: t.color });
    return chips;
  }

  function scopeColor(n: NoteListItem): string {
    return colorOfScope(noteScope(n, sources), sources);
  }

  function relativeTime(iso: string): string {
    const diff = Date.now() - new Date(iso).getTime();
    const m = Math.floor(diff / 60000);
    if (m < 1) return $t('notes_time_just_now');
    if (m < 60) return $t('notes_time_m_ago', { m: String(m) });
    const h = Math.floor(m / 60);
    if (h < 24) return $t('notes_time_h_ago', { h: String(h) });
    return $t('notes_time_d_ago', { d: String(Math.floor(h / 24)) });
  }
</script>

<div class="notes-list">
  {#if notes.length === 0}
    <div class="empty-state">
      <p>{emptyText ?? $t('notes_list_empty')}</p>
      {#if oncreate}
        <button class="btn btn-primary btn-sm" onclick={oncreate}>
          <Icon name="plus" size={13} /> {$t('notes_list_new')}
        </button>
      {/if}
    </div>
  {:else}
    {#each notes as note (note.id)}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
      <div
        class="note-card"
        class:active={note.id === activeId}
        onclick={() => onselect(note.id)}
        role="button"
        tabindex="0"
        onkeydown={(e) => e.key === 'Enter' && onselect(note.id)}
      >
        <div class="card-top">
          <span class="note-title">
            {#if note.pinned}
              <Icon name="pin" size={11} />
            {/if}
            {note.title || $t('notes_untitled')}
          </span>
          <span class="note-meta">
            {#if note.has_draft}
              <span class="draft-dot" title={$t('notes_unsaved_draft')}></span>
            {/if}
            <span class="note-format">{note.format}</span>
          </span>
        </div>

        <div class="card-mid">
          <span class="scope-badge tinted-text" style:--chip={scopeColor(note)}>
            {scopeLabel(note)}
          </span>
          <span class="note-time">{relativeTime(note.updated_at)}</span>
        </div>

        {#if note.snippet}
          <p class="note-preview note-snippet">{@html note.snippet}</p>
        {:else if note.preview}
          <p class="note-preview">{note.preview}</p>
        {/if}

        {#if cardChips(note).length > 0}
          <div class="card-tags">
            {#each cardChips(note).slice(-3) as chip, i (i)}
              <span class="tag-chip tinted" style:--chip={chip.color}>
                {chip.label}
              </span>
            {/each}
            {#if cardChips(note).length > 3}
              <span class="tag-more">+{cardChips(note).length - 3}</span>
            {/if}
          </div>
        {/if}
      </div>
    {/each}
  {/if}
</div>

<style>
  .notes-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .note-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: var(--sp-4);
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s, box-shadow 0.15s;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .note-card:hover { border-color: var(--border-2); }
  /* Active card per design: raised surface + accent ring + soft glow */
  .note-card.active {
    border-color: var(--accent-border);
    background: var(--surface-2);
    box-shadow: var(--shadow-note-active);
  }

  .card-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.4rem;
  }

  .note-title {
    font-size: 0.92rem;
    font-weight: var(--fw-bold);
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: flex;
    align-items: center;
    gap: 0.3rem;
    flex: 1;
    min-width: 0;
  }

  .note-meta {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    flex-shrink: 0;
  }

  .note-format {
    font-size: 0.65rem;
    font-weight: var(--fw-semibold);
    color: var(--text-faint);
    text-transform: uppercase;
    font-family: var(--font-mono);
    background: var(--border);
    padding: 2px 6px;
    border-radius: 5px;
  }

  .draft-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--warn-text);
    flex-shrink: 0;
  }

  .card-mid {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.4rem;
  }

  .scope-badge {
    font-size: var(--fs-2xs);
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 140px;
  }

  .note-time {
    font-size: var(--fs-2xs);
    color: var(--text-3);
    flex-shrink: 0;
  }

  .note-preview {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-2);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    line-height: 1.4;
  }

  .note-snippet :global(mark) {
    background: color-mix(in srgb, var(--accent) 25%, transparent);
    color: var(--text);
    border-radius: 2px;
    padding: 0 1px;
  }

  .card-tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1);
  }

  .tag-chip {
    font-size: var(--fs-xs);
    padding: 3px 9px;
    border-radius: var(--radius-sm);
    border-width: 1px;
    border-style: solid;
    font-weight: var(--fw-semibold);
    white-space: nowrap;
  }

  .tag-more {
    font-size: var(--fs-2xs);
    color: var(--text-2);
    padding: 0.1rem 0.2rem;
  }
</style>

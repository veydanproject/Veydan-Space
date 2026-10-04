<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { api } from '$lib/notes/api';
  import type { NoteListItem } from '$lib/notes/types';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';

  interface Props {
    noteId: string;
    /** Bumped by the editor after a save so backlinks refresh */
    version?: number;
    onopen: (id: string) => void;
    /** Create the missing note for an unresolved `[[target]]` and open it */
    oncreate: (target: string) => void;
  }

  let { noteId, version = 0, onopen, oncreate }: Props = $props();

  let outgoing = $state<NoteListItem[]>([]);
  let backlinks = $state<NoteListItem[]>([]);
  let unresolved = $state<string[]>([]);
  let related = $state<NoteListItem[]>([]);

  $effect(() => {
    const id = noteId;
    void version;
    Promise.all([api.notes.links(id), api.notes.related(id)])
      .then(([l, r]) => {
        if (id !== noteId) return;
        outgoing = l.outgoing;
        backlinks = l.backlinks;
        unresolved = l.unresolved;
        related = r;
      })
      .catch(() => {});
  });
</script>

{#snippet noteGroup(icon: string, label: string, items: NoteListItem[])}
  <div class="group">
    <div class="label"><Icon name={icon} size={11} /> {label} <span class="count">{items.length}</span></div>
    {#if items.length === 0}
      <span class="empty">{$t('note_links_none')}</span>
    {:else}
      {#each items as n (n.id)}
        <button class="item" onclick={() => onopen(n.id)}>
          <span class="title">{n.title}</span>
          {#if n.preview}<span class="preview">{n.preview}</span>{/if}
        </button>
      {/each}
    {/if}
  </div>
{/snippet}

<div class="links">
  {@render noteGroup('arrow-right', $t('note_links_outgoing'), outgoing)}
  {@render noteGroup('arrow-left', $t('note_links_backlinks'), backlinks)}
  {#if unresolved.length > 0}
    <div class="group">
      <div class="label"><Icon name="plus" size={11} /> {$t('note_links_unresolved')} <span class="count">{unresolved.length}</span></div>
      {#each unresolved as target (target)}
        <button class="item" onclick={() => oncreate(target)} title={$t('note_links_create')}>
          <span class="title unresolved">{target}</span>
        </button>
      {/each}
    </div>
  {/if}
  {@render noteGroup('link', $t('note_links_related'), related)}
</div>

<style>
  .links {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: var(--sp-3);
    max-height: min(45%, 200px);
    flex-shrink: 0;
    overflow-y: auto;
    padding: var(--sp-2) var(--sp-4);
    border-top: 1px solid var(--border);
    background: var(--bg-2);
  }
  .group { display: flex; flex-direction: column; gap: 0.2rem; min-width: 0; }
  .label {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: var(--fs-2xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-3);
    font-weight: 600;
    margin-bottom: 0.2rem;
  }
  .count { margin-left: auto; font-weight: 400; }
  .item {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.1rem;
    padding: 0.3rem 0.45rem;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
    min-width: 0;
    width: 100%;
  }
  .item:hover { background: var(--surface-2); }
  .title { font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%; }
  .title.unresolved { color: var(--text-2); text-decoration: underline dotted; }
  .preview { font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%; }
  .empty { font-size: var(--fs-xs); color: var(--text-3); padding: 0.2rem 0.45rem; }
</style>

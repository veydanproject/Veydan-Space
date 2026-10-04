<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { NoteListItem } from '$lib/notes/types';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { binding, isEntityKind, type EntityKind } from '$lib/core/bindings';
  import { entityKind, entityKinds, searchEntities } from '$lib/notes/context';

  interface Props {
    query: string;
    notes: NoteListItem[];
    excludeId?: string | null;
    /** Highlighted row; moved by arrow keys via `handleKeydown` */
    index?: number;
    /** `target` is a note title, or `kind:id` with `label` for an entity mention */
    onpick: (target: string, label?: string | null) => void;
  }

  let { query, notes, excludeId = null, index = $bindable(0), onpick }: Props = $props();

  // A short editor pane caps the list (it scrolls): the row picked by the keys stays in view.
  let listEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    void index;
    listEl?.querySelector('.row.active')?.scrollIntoView({ block: 'nearest' });
  });

  const MAX = 8;

  interface Row {
    /** Unique per row: note id, entity binding or `create` */
    key: string;
    target: string;
    label: string | null;
    icon: string;
    text: string;
    create?: boolean;
  }

  /** `kind:rest` when the query starts with an entity kind prefix. */
  const entityQuery = $derived.by((): { kind: EntityKind; rest: string } | null => {
    const i = query.indexOf(':');
    if (i < 0) return null;
    const kind = query.slice(0, i).trim().toLowerCase();
    return isEntityKind(kind) ? { kind, rest: query.slice(i + 1) } : null;
  });

  const rows = $derived.by((): Row[] => {
    if (entityQuery) {
      const def = entityKind(entityQuery.kind);
      if (!def) return [];
      return searchEntities(entityQuery.kind, entityQuery.rest, MAX).map((e) => {
        const target = binding(entityQuery.kind, e.id);
        return { key: target, target, label: e.name, icon: def.icon, text: e.name };
      });
    }
    const q = query.trim().toLowerCase();
    const live = notes.filter((n) => n.id !== excludeId && !n.deleted);
    const list = q ? live.filter((n) => n.title.toLowerCase().includes(q)) : live;
    const out: Row[] = list
      .sort((a, b) => {
        const as = a.title.toLowerCase().startsWith(q) ? 0 : 1;
        const bs = b.title.toLowerCase().startsWith(q) ? 0 : 1;
        return as - bs || b.updated_at.localeCompare(a.updated_at);
      })
      .slice(0, MAX)
      .map((n) => ({ key: n.id, target: n.title, label: null, icon: 'file-text', text: n.title }));
    if (q && !out.some((r) => r.text.toLowerCase() === q)) {
      out.push({ key: 'create', target: query.trim(), label: null, icon: 'plus', text: $t('note_link_create', { title: query.trim() }), create: true });
    }
    return out;
  });

  /** Kind prefixes offered while nothing has been typed yet. */
  const kindHints = $derived(query.trim().length === 0 ? entityKinds() : []);

  /** Arrow navigation and Enter/Tab pick; returns true when the key was consumed. */
  export function handleKeydown(e: KeyboardEvent): boolean {
    const count = rows.length;
    if (!count) return false;
    if (e.key === 'ArrowDown') { e.preventDefault(); index = (index + 1) % count; return true; }
    if (e.key === 'ArrowUp') { e.preventDefault(); index = (index - 1 + count) % count; return true; }
    if (e.key === 'Enter' || e.key === 'Tab') {
      const row = rows[index];
      if (row) { e.preventDefault(); onpick(row.target, row.label); return true; }
    }
    return false;
  }
</script>

<div class="picker" role="listbox" bind:this={listEl}>
  {#each rows as row, i (row.key)}
    <button class="row" class:create={row.create} class:active={i === index} role="option" aria-selected={i === index} onmousedown={(e) => { e.preventDefault(); onpick(row.target, row.label); }}>
      <Icon name={row.icon} size={12} />
      <span class="title">{row.text}</span>
    </button>
  {/each}
  {#if rows.length === 0}
    <div class="row empty">{entityQuery ? $t('ctx_no_entities') : $t('note_link_type_to_search')}</div>
  {/if}
  {#if kindHints.length > 0}
    <div class="hints">
      {#each kindHints as def (def.kind)}
        <span class="hint"><Icon name={def.icon} size={10} />{def.kind}:</span>
      {/each}
    </div>
  {/if}
</div>

<style>
  .picker {
    position: absolute;
    left: var(--sp-4);
    bottom: var(--sp-3);
    z-index: 5;
    /* Inside the editor pane, whatever its height: the pane clips what sticks out. */
    max-height: calc(100% - 2 * var(--sp-3));
    overflow-y: auto;
    min-width: 240px;
    max-width: 360px;
    padding: 0.25rem;
    background: var(--bg-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    width: 100%;
    padding: 0.35rem 0.5rem;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-sm);
    text-align: left;
    cursor: pointer;
  }
  .row.active, .row:hover { background: var(--surface-2); }
  .row.create { color: var(--accent); }
  .row.empty { color: var(--text-3); cursor: default; }
  .title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hints { display: flex; flex-wrap: wrap; gap: 0.3rem; padding: 0.3rem 0.5rem 0.15rem; border-top: 1px solid var(--border); }
  .hint { display: inline-flex; align-items: center; gap: 0.2rem; font-size: var(--fs-2xs); color: var(--text-3); font-family: var(--font-mono, monospace); }
</style>

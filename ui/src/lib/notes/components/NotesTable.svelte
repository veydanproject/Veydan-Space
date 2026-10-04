<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Table view of a notes list. Columns: fixed note fields plus one per tag prefix (`status/todo` -> `status`); the set is chosen by the user and persisted. -->
<script lang="ts">
  import type { NoteFolder, NoteListItem } from '$lib/notes/types';
  import Icon from '$lib/core/Icon.svelte';
  import { t, locale, type TranslationKey } from '$lib/core/i18n';
  import { relTime } from '$lib/core/utils';
  import { facetKeys, facetValue, facetsOf } from '$lib/notes/facets';
  import { SORT_KEYS, type NoteSort, type SortKey } from '$lib/notes/sort';

  export interface ContextEntity {
    name: string;
    icon: string;
    color: string;
  }

  interface Props {
    /** Rows of the current filter, already ordered by `sort` */
    notes: NoteListItem[];
    /** All live notes; their tag prefixes are the facet columns offered in the picker */
    allNotes: NoteListItem[];
    folders: Pick<NoteFolder, 'id' | 'name'>[];
    /** Shared ordering (title / created / updated), also used by the list view */
    sort: NoteSort;
    onsort: (sort: NoteSort) => void;
    /** Entities behind a note's bindings; platform decides where names come from */
    contextOf: (note: NoteListItem) => ContextEntity[];
    activeId: string | null;
    onselect: (id: string) => void;
    /**
     * The phone: a compact table of the title and the date, rows of finger
     * size and no column picker (a phone has no room for more columns).
     */
    touch?: boolean;
    /** What an empty table says. */
    emptyText?: string;
  }

  let { notes, allNotes, folders, sort, onsort, contextOf, activeId, onselect, touch = false, emptyText }: Props = $props();

  /** Fixed columns; `title` is always shown. */
  const FIXED = ['folder', 'tags', 'context', 'format', 'created_at', 'updated_at', 'draft'] as const;
  type FixedCol = (typeof FIXED)[number];
  type ColKey = 'title' | FixedCol | `facet:${string}`;

  const FIXED_LABEL: Record<FixedCol, TranslationKey> = {
    folder: 'notes_col_folder',
    tags: 'notes_col_tags',
    context: 'ctx_title',
    format: 'notes_col_format',
    created_at: 'notes_col_created',
    updated_at: 'notes_col_updated',
    draft: 'notes_col_draft',
  };

  const DEFAULT_COLS: ColKey[] = ['tags', 'updated_at'];

  // ── Column set: persisted; default = a few fixed ones + 3 most used tag prefixes ──
  const COLS_KEY = 'notes-table-columns';
  function loadCols(): ColKey[] | null {
    try {
      const v = localStorage.getItem(COLS_KEY);
      return v ? (JSON.parse(v) as ColKey[]) : null;
    } catch {
      return null;
    }
  }
  let chosen = $state<ColKey[] | null>(loadCols());
  let colsOpen = $state(false);

  /** Tag prefixes present in the vault, most used first */
  const facets = $derived(facetKeys(allNotes, Infinity));
  const facetCols = $derived(facets.map((k): ColKey => `facet:${k}`));
  const allCols = $derived<ColKey[]>([...FIXED, ...facetCols]);
  const cols = $derived<ColKey[]>(
    touch
      ? ['updated_at']
      : chosen
        ? chosen.filter((c) => allCols.includes(c))
        : [...DEFAULT_COLS, ...facetCols.slice(0, 3)],
  );

  function toggleCol(col: ColKey) {
    // Keep the picker's order so columns do not jump around
    const next = cols.includes(col) ? cols.filter((c) => c !== col) : allCols.filter((c) => c === col || cols.includes(c));
    chosen = next;
    try { localStorage.setItem(COLS_KEY, JSON.stringify(next)); } catch {}
  }

  function colLabel(col: ColKey): string {
    if (col === 'title') return $t('notes_col_title');
    if (col.startsWith('facet:')) return col.slice('facet:'.length);
    return $t(FIXED_LABEL[col as FixedCol]);
  }

  // ── Cell values ──
  const folderName = (id: string) => folders.find((f) => f.id === id)?.name ?? '';
  const plainTags = (n: NoteListItem) => n.tags.filter((tg) => !facetsOf([tg]).length);

  const context = (n: NoteListItem) => contextOf(n);

  /** Comparable string for sorting */
  function sortValue(n: NoteListItem, col: ColKey): string {
    switch (col) {
      case 'title': return n.title.toLowerCase();
      case 'folder': return n.folder_ids.map(folderName).join(', ').toLowerCase();
      case 'tags': return plainTags(n).map((tg) => tg.name).join(', ').toLowerCase();
      case 'context': return context(n).map((c) => c.name).join(', ').toLowerCase();
      case 'format': return n.format;
      case 'created_at': return n.created_at;
      case 'updated_at': return n.updated_at;
      case 'draft': return n.has_draft ? '1' : '';
      default: return facetValue(n.tags, col.slice('facet:'.length))?.value.toLowerCase() ?? '';
    }
  }

  // Title / created / updated go through the shared sort; other columns are ordered here only
  const isShared = (col: ColKey): col is SortKey => (SORT_KEYS as readonly string[]).includes(col);
  let local = $state<{ col: ColKey; asc: boolean } | null>(null);
  // The header sort control wins over a column-local order
  $effect(() => { void sort.key; void sort.asc; local = null; });

  const sortKey = $derived<ColKey>(local ? local.col : sort.key);
  const sortAsc = $derived(local ? local.asc : sort.asc);

  const sorted = $derived.by(() => {
    // Bound to a const: the narrowing from the guard below does not survive into
    // the comparator closure, since `local` is a mutable outer binding.
    const order = local;
    if (!order) return notes;
    const dir = order.asc ? 1 : -1;
    return [...notes].sort((a, b) => {
      if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
      const av = sortValue(a, order.col);
      const bv = sortValue(b, order.col);
      // Empty cells sink to the bottom regardless of direction
      if (!av !== !bv) return av ? -1 : 1;
      return av.localeCompare(bv) * dir;
    });
  });

  function toggleSort(col: ColKey) {
    const asc = sortKey === col ? !sortAsc : !['updated_at', 'created_at'].includes(col);
    if (isShared(col)) {
      local = null;
      onsort({ key: col, asc });
    } else {
      local = { col, asc };
    }
  }
</script>

{#snippet header(col: ColKey)}
  <th>
    <button class="th" class:on={sortKey === col} onclick={() => toggleSort(col)}>
      {colLabel(col)}
      {#if sortKey === col}<Icon name={sortAsc ? 'arrow-up' : 'arrow-down'} size={10} />{/if}
    </button>
  </th>
{/snippet}

{#snippet cell(n: NoteListItem, col: ColKey)}
  {#if col === 'folder'}
    <td class="muted">{n.folder_ids.map(folderName).filter(Boolean).join(', ')}</td>
  {:else if col === 'tags'}
    <td>
      <div class="chips">
        {#each plainTags(n) as tg (tg.id)}
          <span class="chip tinted" style:--chip={tg.color}>{tg.name}</span>
        {/each}
      </div>
    </td>
  {:else if col === 'context'}
    <!-- Kind icons only; the name is in the tooltip to keep the row quiet -->
    <td>
      <div class="chips">
        {#each context(n) as c, i (i)}
          <span class="entity" style="color:{c.color}" title={c.name}><Icon name={c.icon} size={12} /></span>
        {/each}
      </div>
    </td>
  {:else if col === 'format'}
    <td class="muted mono">{n.format.toUpperCase()}</td>
  {:else if col === 'created_at'}
    <td class="muted">{relTime(n.created_at, $locale)}</td>
  {:else if col === 'updated_at'}
    <td class="muted">{relTime(n.updated_at, $locale)}</td>
  {:else if col === 'draft'}
    <td class="muted">{#if n.has_draft}<span class="draft">{$t('panel_notes_draft')}</span>{/if}</td>
  {:else}
    {@const f = facetValue(n.tags, col.slice('facet:'.length))}
    <td>
      {#if f}<span class="chip tinted" style:--chip={f.color}>{f.value}</span>{/if}
    </td>
  {/if}
{/snippet}

<svelte:window onclick={() => (colsOpen = false)} />

<div class="wrap" class:touch>
  <table>
    <thead>
      <tr>
        {@render header('title')}
        {#each cols as col (col)}
          {@render header(col)}
        {/each}
        {#if !touch}
        <th class="cols-th">
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <span class="cols" onclick={(e) => e.stopPropagation()}>
            <button class="th" title={$t('notes_table_columns')} onclick={() => (colsOpen = !colsOpen)}>
              <Icon name="columns" size={11} />
            </button>
            {#if colsOpen}
              <div class="cols-menu">
                <div class="cols-group">{$t('notes_table_columns_fields')}</div>
                {#each FIXED as col (col)}
                  <label class="cols-item">
                    <input type="checkbox" checked={cols.includes(col)} onchange={() => toggleCol(col)} />
                    <span>{colLabel(col)}</span>
                  </label>
                {/each}
                <div class="cols-group">{$t('notes_table_columns_facets')}</div>
                <div class="cols-hint">{$t('notes_table_columns_hint')}</div>
                {#if facetCols.length === 0}
                  <div class="cols-hint">{$t('notes_table_columns_none')}</div>
                {/if}
                {#each facetCols as col (col)}
                  <label class="cols-item">
                    <input type="checkbox" checked={cols.includes(col)} onchange={() => toggleCol(col)} />
                    <span class="mono">{colLabel(col)}/</span>
                  </label>
                {/each}
              </div>
            {/if}
          </span>
        </th>
        {/if}
      </tr>
    </thead>
    <tbody>
      {#each sorted as n (n.id)}
        <tr class:active={n.id === activeId} onclick={() => onselect(n.id)}>
          <td>
            <div class="title">
              {#if n.pinned}<Icon name="pin" size={10} />{/if}
              <span>{n.title || $t('notes_untitled')}</span>
            </div>
          </td>
          {#each cols as col (col)}
            {@render cell(n, col)}
          {/each}
          {#if !touch}<td></td>{/if}
        </tr>
      {/each}
    </tbody>
  </table>
  {#if notes.length === 0}
    <div class="empty">{emptyText ?? $t('panel_notes_empty')}</div>
  {/if}
</div>

<style>
  .wrap { overflow: auto; }
  table { width: 100%; border-collapse: collapse; font-size: var(--fs-sm); }
  th { text-align: left; padding: 0 0.4rem 0.3rem; border-bottom: 1px solid var(--border); white-space: nowrap; }
  .th {
    display: inline-flex; align-items: center; gap: 0.25rem;
    background: none; border: 0; padding: 0; cursor: pointer;
    font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.06em;
    color: var(--text-3); font-weight: 600;
  }
  .th.on { color: var(--text); }
  td { padding: 0.35rem 0.4rem; border-bottom: 1px solid var(--border); vertical-align: middle; }
  tbody tr { cursor: pointer; }
  tbody tr:hover { background: var(--surface-2); }
  tbody tr.active { background: color-mix(in srgb, var(--accent) 12%, transparent); }
  /* Flex only on wrappers: a flex `td` drops out of the table layout */
  .title { display: flex; align-items: center; gap: 0.35rem; min-width: 0; max-width: 320px; }
  .title span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .muted { font-size: var(--fs-2xs); color: var(--text-3); white-space: nowrap; }
  .draft { color: var(--warn-text); }
  .chips { display: flex; flex-wrap: wrap; gap: 0.3rem; }
  .chip {
    display: inline-block; padding: 0.1rem 0.45rem; border-width: 1px; border-style: solid; border-radius: 999px;
    font-size: var(--fs-2xs); white-space: nowrap;
  }
  .entity { display: inline-flex; align-items: center; }
  .empty { padding: var(--sp-4); text-align: center; color: var(--text-3); font-size: var(--fs-sm); }
  .cols-th { width: 1.6rem; text-align: right; }
  .cols { position: relative; display: inline-block; }

  /* Desktop: the columns keep their width and the table scrolls sideways under a pinned title */
  .wrap:not(.touch) td, .wrap:not(.touch) th { white-space: nowrap; }
  .wrap:not(.touch) th:first-child, .wrap:not(.touch) td:first-child {
    position: sticky; left: 0; z-index: 1;
    background: var(--surface);
  }
  .wrap:not(.touch) tbody tr:hover td:first-child { background: var(--surface-2); }
  .wrap:not(.touch) tbody tr.active td:first-child { background: color-mix(in srgb, var(--accent) 12%, var(--surface)); }

  /* Touch: two columns, the title takes the room and is cut with an ellipsis; rows of finger size */
  .touch table { table-layout: fixed; font-size: var(--fs-md); }
  .touch th, .touch td { padding: 0 var(--sp-2); height: 48px; }
  .touch th { height: 36px; }
  .touch th:last-child, .touch td:last-child { width: 38%; text-align: right; }
  .touch th:last-child .th { justify-content: flex-end; }
  .touch .title { max-width: none; }
  .touch .muted { font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; }
  .touch .th { font-size: var(--fs-2xs); }
  .cols-menu {
    position: absolute; right: 0; top: calc(100% + 4px); z-index: 20;
    min-width: 200px; max-height: 60vh; overflow-y: auto; padding: 0.4rem;
    background: var(--bg-2); border: 1px solid var(--border); border-radius: var(--radius-sm);
    box-shadow: var(--shadow-lg);
    display: flex; flex-direction: column; gap: 0.15rem;
    text-transform: none; letter-spacing: 0;
  }
  .cols-group {
    padding: 0.3rem 0.3rem 0.1rem; font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.06em;
    color: var(--text-3); font-weight: 600;
  }
  .cols-hint { font-size: var(--fs-2xs); color: var(--text-3); padding: 0 0.3rem 0.2rem; white-space: normal; font-weight: 400; }
  .cols-item { display: flex; align-items: center; gap: 0.4rem; padding: 0.2rem 0.3rem; font-size: var(--fs-sm); color: var(--text); cursor: pointer; font-weight: 400; }
  .cols-item input[type="checkbox"] { margin: 0; }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t, locale } from '$lib/core/i18n';
  import { formatBytes, formatDateTime } from '$lib/core/utils';
  import type { FileEntry } from '$lib/ssh/types';
  import type { SortKey } from '$lib/ssh/store/sftp.svelte';

  interface Props {
    entries: FileEntry[];
    selected: string[];
    sortKey: SortKey;
    sortDir: 1 | -1;
    onsort: (key: SortKey) => void;
    onopen: (entry: FileEntry) => void;
    onselect: (entry: FileEntry, mods: { ctrl: boolean; shift: boolean }) => void;
    oncontextmenu: (entry: FileEntry, x: number, y: number) => void;
    onemptycontextmenu: (x: number, y: number) => void;
  }

  let { entries, selected, sortKey, sortDir, onsort, onopen, onselect, oncontextmenu, onemptycontextmenu }: Props = $props();

  const columns: { key: SortKey; labelKey: 'files_col_name' | 'files_col_size' | 'files_col_mtime' | 'files_col_perms'; cls: string }[] = [
    { key: 'name', labelKey: 'files_col_name', cls: 'col-name' },
    { key: 'size', labelKey: 'files_col_size', cls: 'col-size' },
    { key: 'mtime', labelKey: 'files_col_mtime', cls: 'col-mtime' },
    { key: 'permissions', labelKey: 'files_col_perms', cls: 'col-perms' },
  ];
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="file-table-wrap"
  oncontextmenu={(e) => {
    // Right-click on empty space (not a row) → panel-level menu
    if ((e.target as HTMLElement).closest('.file-row')) return;
    e.preventDefault();
    onemptycontextmenu(e.clientX, e.clientY);
  }}
>
  <table class="file-table">
    <thead>
      <tr>
        {#each columns as col (col.key)}
          <th class={col.cls}>
            <button class="sort-btn" onclick={() => onsort(col.key)}>
              {$t(col.labelKey)}
              {#if sortKey === col.key}
                <Icon name={sortDir === 1 ? 'chevron-down' : 'arrow-up'} size={11} />
              {/if}
            </button>
          </th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each entries as entry (entry.path)}
        <tr
          class="file-row"
          class:selected={selected.includes(entry.path)}
          onclick={(e) => onselect(entry, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey })}
          ondblclick={() => onopen(entry)}
          oncontextmenu={(e) => { e.preventDefault(); oncontextmenu(entry, e.clientX, e.clientY); }}
        >
          <td class="col-name">
            <span class="name-cell" class:dir={entry.is_dir}>
              <Icon name={entry.is_dir ? 'folder' : 'file'} size={14} strokeWidth={1.8} />
              <span class="entry-name">{entry.name}</span>
              {#if entry.is_symlink}<span class="symlink-mark" title={$t('files_symlink')}>→</span>{/if}
            </span>
          </td>
          <td class="col-size">
            {#if entry.is_dir}<span class="text-muted">—</span>{:else}{formatBytes(entry.size)}{/if}
          </td>
          <td class="col-mtime">{formatDateTime(entry.mtime, $locale)}</td>
          <td class="col-perms"><code class="perms">{entry.permissions}</code></td>
        </tr>
      {/each}
    </tbody>
  </table>

  {#if entries.length === 0}
    <div class="empty-dir">{$t('files_empty_dir')}</div>
  {/if}
</div>

<style>
  .file-table-wrap {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .file-table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
    font-size: var(--fs-sm);
  }

  thead th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    text-align: left;
    padding: 0;
  }

  .sort-btn {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    width: 100%;
    padding: var(--sp-2) var(--sp-3);
    background: none;
    border: none;
    color: var(--text-2);
    font-size: var(--fs-xs);
    font-weight: var(--fw-semibold);
    text-transform: uppercase;
    letter-spacing: 0.4px;
    cursor: pointer;
  }
  .sort-btn:hover { color: var(--text); }

  .col-size { width: 90px; }
  /* "10/14/2026, 11:55 PM" at the table's font with the cell padding */
  .col-mtime { width: 168px; }
  .col-perms { width: 110px; }

  .file-row {
    cursor: default;
    user-select: none;
  }
  .file-row td {
    padding: var(--sp-1) var(--sp-3);
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-2);
  }
  .file-row:hover td { background: var(--surface-hover); }
  .file-row.selected td { background: var(--accent-bg); }

  .name-cell {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
    color: var(--text);
  }
  .name-cell :global(svg) { flex-shrink: 0; color: var(--text-3); }
  .name-cell.dir :global(svg) { color: var(--accent-text); }
  .entry-name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .symlink-mark { color: var(--text-3); flex-shrink: 0; }

  .perms {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-3);
    background: none;
    padding: 0;
  }

  .text-muted { color: var(--text-3); }

  .empty-dir {
    padding: var(--sp-8) var(--sp-4);
    text-align: center;
    color: var(--text-3);
    font-size: var(--fs-sm);
  }
</style>

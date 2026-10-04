<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { api } from '$lib/notes/api';
  import type { NoteHistoryEntry, DiffLine, HistoryFilter, VersionType } from '$lib/notes/types';
  import Icon from '$lib/core/Icon.svelte';
  import { locale, t, type TranslationKey } from '$lib/core/i18n';
  import { relTime } from '$lib/core/utils';

  interface Props {
    noteId: string;
    onrestore: (note: import('$lib/notes/types').Note) => void;
    onmerge: (historyId: string) => void;
    onclose: () => void;
  }

  let { noteId, onrestore, onmerge, onclose }: Props = $props();

  type ViewMode = 'list' | 'diff';

  let entries = $state<NoteHistoryEntry[]>([]);
  let loading = $state(false);
  let error = $state('');

  let selectedA = $state<NoteHistoryEntry | null>(null);
  let selectedB = $state<NoteHistoryEntry | null>(null);
  let compareMode = $state(false);

  let diffLines = $state<DiffLine[]>([]);
  let diffLoading = $state(false);
  let viewMode = $state<ViewMode>('list');

  let filter = $state<HistoryFilter>({});
  let filterType = $state<VersionType | ''>('');
  let filterDateFrom = $state('');
  let filterDateTo = $state('');

  let restoring = $state(false);
  let merging = $state(false);

  $effect(() => {
    // Tracks noteId explicitly: another note resets the state and reloads
    const _id = noteId;
    selectedA = null;
    selectedB = null;
    compareMode = false;
    viewMode = 'list';
    diffLines = [];
    loadHistory();
  });

  async function loadHistory() {
    loading = true;
    error = '';
    try {
      const f: HistoryFilter = {};
      if (filterType) f.version_type = filterType as VersionType;
      if (filterDateFrom) f.date_from = filterDateFrom;
      if (filterDateTo) f.date_to = filterDateTo;
      entries = await api.notes.historyList(noteId, f);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  function applyFilter() {
    filter = {};
    if (filterType) filter.version_type = filterType as VersionType;
    if (filterDateFrom) filter.date_from = filterDateFrom;
    if (filterDateTo) filter.date_to = filterDateTo;
    selectedA = null;
    selectedB = null;
    compareMode = false;
    viewMode = 'list';
    loadHistory();
  }

  function clearFilter() {
    filterType = '';
    filterDateFrom = '';
    filterDateTo = '';
    filter = {};
    loadHistory();
  }

  function selectEntry(entry: NoteHistoryEntry) {
    if (compareMode) {
      if (!selectedA) {
        selectedA = entry;
      } else if (!selectedB && entry.id !== selectedA.id) {
        selectedB = entry;
        showDiff();
      } else {
        selectedA = entry;
        selectedB = null;
        viewMode = 'list';
        diffLines = [];
      }
    } else {
      selectedA = entry;
      selectedB = null;
      showDiff(); // straight into the diff
    }
  }

  function backToList() {
    viewMode = 'list';
    selectedA = null;
    selectedB = null;
    diffLines = [];
  }

  async function showDiff() {
    if (!selectedA) return;
    diffLoading = true;
    viewMode = 'diff';
    try {
      const fromId = selectedB ? selectedB.id : selectedA.id;
      const toId = selectedB ? selectedA.id : 'current';
      const result = await api.notes.historyDiff(noteId, fromId, toId);
      diffLines = result.lines;
    } catch (e) {
      error = String(e);
    } finally {
      diffLoading = false;
    }
  }

  async function restore() {
    if (!selectedA) return;
    restoring = true;
    try {
      const note = await api.notes.historyRestore(noteId, selectedA.id);
      onrestore(note);
      await loadHistory();
      selectedA = null;
      viewMode = 'list';
    } catch (e) {
      error = String(e);
    } finally {
      restoring = false;
    }
  }

  function startMerge() {
    if (!selectedA) return;
    onmerge(selectedA.id);
  }

  const formatDate = (iso: string): string => relTime(iso, $locale);

  function dateGroupLabel(iso: string): string {
    const d = new Date(iso);
    const now = new Date();
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    const yesterday = new Date(today.getTime() - 86400000);
    const entryDay = new Date(d.getFullYear(), d.getMonth(), d.getDate());
    if (entryDay.getTime() === today.getTime()) return $t('notes_history_today');
    if (entryDay.getTime() === yesterday.getTime()) return $t('notes_history_yesterday');
    return d.toLocaleDateString($locale, { day: 'numeric', month: 'long', year: 'numeric' });
  }

  // Group entries by day
  const grouped = $derived.by(() => {
    const groups: { label: string; items: NoteHistoryEntry[] }[] = [];
    let lastLabel = '';
    for (const e of entries) {
      const label = dateGroupLabel(e.created_at);
      if (label !== lastLabel) {
        groups.push({ label, items: [] });
        lastLabel = label;
      }
      groups[groups.length - 1].items.push(e);
    }
    return groups;
  });

  const versionTypeIcon: Record<string, string> = {
    save: 'save',
    autosave: 'clock',
    restore: 'rotate-ccw',
    merge: 'git-merge',
    import: 'upload',
    sync: 'refresh-cw',
    conflict: 'alert-triangle',
  };

  const VERSION_TYPES = ['save', 'autosave', 'restore', 'merge', 'import', 'sync', 'conflict'] as const;
  const versionTypeLabel: Record<string, TranslationKey> = {
    save: 'notes_history_kind_save',
    autosave: 'notes_history_kind_autosave',
    restore: 'notes_history_kind_restore',
    merge: 'notes_history_kind_merge',
    import: 'notes_history_kind_import',
    sync: 'notes_history_kind_sync',
    conflict: 'notes_history_kind_conflict',
  };

  const diffCountAdded = $derived(diffLines.filter(l => l.kind === 'added').length);
  const diffCountRemoved = $derived(diffLines.filter(l => l.kind === 'removed').length);
</script>

<div class="history-panel">
  <div class="panel-header">
    <span class="panel-title">
      <Icon name="clock" size={14} />
      {$t('notes_history_title')}
    </span>
    <div class="header-actions">
      <button
        class="btn-mode"
        class:active={compareMode}
        onclick={() => { compareMode = !compareMode; selectedB = null; }}
        title={$t('notes_history_compare')}
      >
        <Icon name="columns" size={13} />
      </button>
      <button class="btn-close" onclick={onclose} title={$t('common_close')}>
        <Icon name="x" size={14} />
      </button>
    </div>
  </div>

  <!-- Filter bar -->
  <!-- The type on a row of its own with the buttons, the two dates under it: a narrow panel cuts nothing -->
  <div class="hist-filter">
    <div class="filter-row">
      <select bind:value={filterType} class="filter-select" aria-label={$t('notes_history_all_types')}>
        <option value="">{$t('notes_history_all_types')}</option>
        {#each VERSION_TYPES as kind (kind)}
          <option value={kind}>{$t(versionTypeLabel[kind])}</option>
        {/each}
      </select>
      <button class="btn-filter" onclick={applyFilter} title={$t('notes_history_apply')} aria-label={$t('notes_history_apply')}>
        <Icon name="filter" size={12} />
      </button>
      {#if filterType || filterDateFrom || filterDateTo}
        <button class="btn-filter btn-clear" onclick={clearFilter} title={$t('notes_history_reset')} aria-label={$t('notes_history_reset')}>
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>
    <div class="filter-dates">
      <input type="date" bind:value={filterDateFrom} class="filter-date" title={$t('notes_history_from')} aria-label={$t('notes_history_from')} />
      <input type="date" bind:value={filterDateTo} class="filter-date" title={$t('notes_history_to')} aria-label={$t('notes_history_to')} />
    </div>
  </div>

  {#if error}
    <div class="error-bar">{error}</div>
  {/if}

  <!-- Compare mode hint -->
  {#if compareMode && viewMode !== 'diff'}
    <div class="compare-hint">
      {#if !selectedA}
        {$t('notes_history_pick_first')}
      {:else if !selectedB}
        {$t('notes_history_pick_second')}
      {/if}
    </div>
  {/if}

  {#if viewMode === 'diff'}
    <div class="diff-header">
      <button class="btn-back" onclick={backToList} title={$t('notes_history_back')}>
        <Icon name="arrow-left" size={12} /> {$t('notes_history_versions')}
      </button>
      <div class="diff-header-right">
        {#if diffLines.length > 0}
          <span class="diff-stats">
            <span class="stat-added">+{diffCountAdded}</span>
            <span class="stat-removed">-{diffCountRemoved}</span>
          </span>
        {/if}
        {#if selectedA && !selectedB}
          <button class="btn-action" onclick={startMerge} title={$t('notes_history_merge')}>
            <Icon name="git-merge" size={13} />
          </button>
          <button class="btn-action btn-restore" onclick={restore} disabled={restoring} title={$t('notes_history_revert')}>
            <Icon name="rotate-ccw" size={13} />
          </button>
        {/if}
      </div>
    </div>
    <div class="diff-view">
      {#if diffLoading}
        <div class="loading-center">
          <Icon name="loader" size={16} />
        </div>
      {:else if diffLines.length === 0}
        <div class="empty-diff">{$t('notes_history_no_changes')}</div>
      {:else}
        {#each diffLines as line}
          <div class="diff-line diff-{line.kind}">
            <span class="diff-marker">
              {line.kind === 'added' ? '+' : line.kind === 'removed' ? '-' : ' '}
            </span>
            <span class="diff-content">{line.content}</span>
          </div>
        {/each}
      {/if}
    </div>
  {:else}
    <div class="entries-list">
      {#if loading}
        <div class="loading-center">
          <Icon name="loader" size={16} />
        </div>
      {:else if entries.length === 0}
        <div class="empty-state">
          <span class="empty-icon"><Icon name="clock" size={22} /></span>
          <p>{$t('notes_history_none')}</p>
          <span>{$t('notes_history_none_hint')}</span>
        </div>
      {:else}
        {#each grouped as group}
          <div class="day-group">
            <div class="day-label">{group.label}</div>
            {#each group.items as entry}
              {@const isSelectedA = selectedA?.id === entry.id}
              {@const isSelectedB = selectedB?.id === entry.id}
              <button
                class="entry"
                class:selected-a={isSelectedA}
                class:selected-b={isSelectedB}
                onclick={() => selectEntry(entry)}
              >
                <span class="entry-icon">
                  <Icon name={versionTypeIcon[entry.version_type] ?? 'save'} size={12} />
                </span>
                <div class="entry-info">
                  <span class="entry-type">{versionTypeLabel[entry.version_type] ? $t(versionTypeLabel[entry.version_type]) : entry.version_type}</span>
                  <span class="entry-time">{formatDate(entry.created_at)}</span>
                </div>
                <span class="entry-rev">r{entry.revision}</span>
              </button>
            {/each}
          </div>
        {/each}
      {/if}
    </div>
  {/if}
</div>

<style>
  .history-panel {
    width: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    overflow: hidden;
    font-size: var(--fs-sm);
    container-type: inline-size;
  }

  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--sp-2) var(--sp-3);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .panel-title {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--text-2);
  }

  .header-actions {
    display: flex;
    gap: 0.2rem;
  }

  .btn-close, .btn-mode {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-3);
    padding: 0.2rem;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    transition: color 0.15s, background 0.15s;
  }

  .btn-close:hover, .btn-mode:hover { color: var(--text); background: var(--surface-hover); }
  .btn-mode.active { color: var(--accent-text); }

  /* Not the global .filter-bar of base.css (a centred row) */
  .hist-filter {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    padding: 0.4rem 0.6rem;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .filter-row { display: flex; align-items: center; gap: 0.3rem; min-width: 0; }
  .filter-dates { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 0.3rem; }
  /* A narrow panel stacks the dates: each one whole. */
  @container (max-width: 280px) {
    .filter-dates { grid-template-columns: minmax(0, 1fr); }
  }

  .filter-select {
    font-size: var(--fs-xs);
    /* background-color only: the shorthand would drop the chevron of base.css */
    background-color: var(--surface-3);
    background-position: right 0.4rem center;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text);
    padding: 0 1.4rem 0 0.4rem;
    height: 26px;
    flex: 1;
    min-width: 0;
  }

  .filter-date {
    font-size: var(--fs-2xs);
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text);
    padding: 0.15rem 0.25rem;
    width: 100%;
    min-width: 0;
  }

  .btn-filter {
    background: none;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 0.2rem 0.35rem;
    cursor: pointer;
    color: var(--text-2);
    display: flex;
    align-items: center;
    transition: background 0.15s;
    flex-shrink: 0;
  }

  .btn-filter:hover { background: var(--surface-hover); color: var(--text); }
  .btn-clear { color: var(--danger-text); }

  .compare-hint {
    font-size: var(--fs-2xs);
    color: var(--accent-text);
    background: var(--accent-tint);
    padding: 0.3rem var(--sp-3);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    text-align: center;
  }

  .error-bar {
    background: var(--danger-bg);
    color: var(--danger-text);
    font-size: var(--fs-xs);
    padding: 0.3rem var(--sp-3);
    flex-shrink: 0;
  }

  .btn-action {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    background: none;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 0.15rem var(--sp-2);
    font-size: var(--fs-2xs);
    color: var(--text-2);
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }

  .btn-action:hover { background: var(--surface-hover); color: var(--text); }
  .btn-restore { border-color: var(--accent-border); color: var(--accent-text); }
  .btn-restore:hover { background: var(--accent-tint); }
  .btn-restore:disabled { opacity: 0.5; cursor: not-allowed; }

  .diff-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.35rem var(--sp-2) 0.35rem 0.6rem;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    gap: 0.4rem;
  }

  .diff-header-right {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-shrink: 0;
  }

  .btn-back {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    background: none;
    border: none;
    cursor: pointer;
    font-size: var(--fs-xs);
    color: var(--text-2);
    padding: 0;
  }

  .btn-back:hover { color: var(--text); }

  .diff-stats {
    display: flex;
    gap: var(--sp-2);
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
  }

  .stat-added { color: var(--success-text); }
  .stat-removed { color: var(--danger-text); }

  .diff-view {
    flex: 1;
    overflow-y: auto;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.5;
  }

  .diff-line {
    display: flex;
    gap: 0.3rem;
    padding: 0 var(--sp-2);
    white-space: pre-wrap;
    word-break: break-all;
  }

  .diff-context { color: var(--text-3); }
  .diff-added { background: var(--success-bg); color: var(--success-text); }
  .diff-removed { background: var(--danger-bg); color: var(--danger-text); }

  .diff-marker {
    flex-shrink: 0;
    width: 1ch;
    user-select: none;
  }

  .diff-content { flex: 1; min-width: 0; }

  .entries-list {
    flex: 1;
    overflow-y: auto;
    padding: var(--sp-1) 0;
  }

  .loading-center {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--sp-8);
    color: var(--text-3);
  }

  .empty-diff {
    padding: var(--sp-8);
    text-align: center;
    color: var(--text-3);
  }

  .day-group { padding-bottom: var(--sp-1); }

  .day-label {
    font-size: var(--fs-2xs);
    font-weight: 600;
    color: var(--text-3);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 0.4rem var(--sp-3) 0.2rem;
    position: sticky;
    top: 0;
    background: var(--surface);
    z-index: 1;
  }

  .entry {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    width: 100%;
    background: none;
    border: none;
    padding: 0.35rem var(--sp-3);
    cursor: pointer;
    text-align: left;
    border-radius: 0;
    transition: background 0.12s;
    color: var(--text);
  }

  .entry:hover { background: var(--surface-row-hover); }
  .entry.selected-a { background: var(--accent-bg); }
  .entry.selected-b { background: var(--success-bg); }

  .entry-icon {
    color: var(--text-3);
    flex-shrink: 0;
    display: flex;
    align-items: center;
  }

  .entry-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.05rem;
  }

  .entry-type {
    font-size: var(--fs-sm);
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .entry-time {
    font-size: var(--fs-2xs);
    color: var(--text-3);
  }

  .entry-rev {
    font-size: var(--fs-2xs);
    color: var(--text-3);
    font-family: var(--font-mono);
    flex-shrink: 0;
  }
</style>

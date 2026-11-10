<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { notesStore } from '$lib/notes/store/notes.svelte';
  import { directory } from '$lib/core/directory';
  import { api } from '$lib/notes/api';
  import { isNotesWindow } from '$lib/notes/window';
  import type { NoteCreateInput } from '$lib/notes/types';
  import { toNoteFilter, contextBindings, templateNotes, type ActiveFilter } from '$lib/notes/filter';
  import TemplateSelect from '$lib/notes/components/TemplateSelect.svelte';
  import { appLock } from '$lib/core/lock/store.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import NotesList from '$lib/notes/components/NotesList.svelte';
  import NotesTable from '$lib/notes/components/NotesTable.svelte';
  import { paletteStore } from '$lib/core/store/palette.svelte';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
  import { noteContextEntities } from '$lib/notes/context';
  import { loadSort, saveSort, sortNotes, SORT_KEYS, type NoteSort, type SortKey } from '$lib/notes/sort';
  import ListBulkActions from '$lib/notes/components/ListBulkActions.svelte';
  import NoteEditor from '$lib/notes/components/NoteEditor.svelte';
  import NoteFilters from '$lib/notes/components/NoteFilters.svelte';
  import NoteTransferDialog, { type TransferMode } from '$lib/notes/components/NoteTransferDialog.svelte';
  import { t, locale, countKey } from '$lib/core/i18n';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import NotesSidebarFooter from '$lib/notes/components/NotesSidebarFooter.svelte';
  import { notesListTitle } from '$lib/notes/filter-title';

  const standaloneNotes = isNotesWindow();

  let searchQuery = $state('');
  let searching = $state(false);
  let showCreate = $state(false);
  let createTitle = $state('');
  let createTemplate = $state('');
  const hasTemplates = $derived(templateNotes(notesStore.list, notesStore.folders).length > 0);
  let activeFilter = $state<ActiveFilter>({ type: 'all' });
  let sidebarVisible = $state(true);

  // List or table view of the middle column
  const VIEW_KEY = 'notes-view-mode';
  let viewMode = $state<'list' | 'table'>(localStorage.getItem(VIEW_KEY) === 'table' ? 'table' : 'list');
  function setViewMode(m: 'list' | 'table') {
    viewMode = m;
    try { localStorage.setItem(VIEW_KEY, m); } catch {}
  }

  // Resizable columns; the table has a width of its own: its columns need more room than the cards.
  // The sidebar is a share of the page but never under SIDEBAR_MIN: dragged narrower, it stops there.
  // It is hidden only by «Свернуть панель» and comes back by «Показать панель» in the list's header.
  const SIDEBAR_MIN = 200;
  const COL_DEFAULTS = { sidebar: 20, list: 27, table: 50 };
  function loadColWidths(): { sidebar: number; list: number; table: number } {
    try {
      const v = localStorage.getItem('notes-col-widths');
      if (v) return { ...COL_DEFAULTS, ...JSON.parse(v) };
    } catch {}
    return { ...COL_DEFAULTS };
  }

  let colWidths = $state(loadColWidths());
  let pageBodyEl: HTMLElement | null = $state(null);
  let dragging: 'sidebar' | 'list' | null = $state(null);
  let hoveredResizer: 'sidebar' | 'list' | null = $state(null);

  function saveColWidths() {
    try { localStorage.setItem('notes-col-widths', JSON.stringify(colWidths)); } catch {}
  }

  function startColDrag(e: MouseEvent, col: 'sidebar' | 'list') {
    e.preventDefault();
    dragging = col;

    function onMove(ev: MouseEvent) {
      if (!pageBodyEl) return;
      const rect = pageBodyEl.getBoundingClientRect();
      const pct = ((ev.clientX - rect.left) / rect.width) * 100;
      if (col === 'sidebar') {
        colWidths.sidebar = Math.max((SIDEBAR_MIN / rect.width) * 100, Math.min(30, pct));
      } else if (viewMode === 'table') {
        colWidths.table = Math.max(25, Math.min(70, pct - (sidebarVisible ? colWidths.sidebar : 0)));
      } else {
        colWidths.list = Math.max(15, Math.min(45, pct - (sidebarVisible ? colWidths.sidebar : 0)));
      }
      saveColWidths();
    }

    function onUp() {
      dragging = null;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    }

    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }

  onMount(() => {
    notesStore.ensureLoaded();
    void directory.ensureLoaded();
    notesStore.refreshTrash();
    notesStore.startWatcher();
    // The store is shared: a notes drawer (a workspace's, a profile's) may have
    // left its own filter in it. The page shows what its sidebar says.
    void notesStore.setFilter(toNoteFilter(activeFilter));
    // Note id passed by the browser extension when this window was just created
    const requested = new URLSearchParams(window.location.search).get('open');
    if (requested) notesStore.openRequestId = requested;
  });

  const isTrash = $derived(activeFilter.type === 'trash');

  // Ordering shared by list and table views; FTS results keep their relevance order
  let sort = $state<NoteSort>(loadSort());
  const isFtsSearch = $derived(searchQuery.trim().length >= 2 && !isTrash);
  function setSort(next: NoteSort) {
    sort = next;
    saveSort(next);
  }
  // Filtering happens on the backend (notesStore.view); only the 1-char
  // search (too short for FTS) and trash search are applied locally.
  const displayList = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    const list = notesStore.view;
    if (isFtsSearch) return list;
    const filtered = !q ? list : list.filter(
      (n) =>
        n.title.toLowerCase().includes(q) ||
        n.tags.some((tg: { name: string }) => tg.name.toLowerCase().includes(q)) ||
        n.folder_ids.some(fid => notesStore.folders.find(f => f.id === fid)?.name.toLowerCase().includes(q))
    );
    return sortNotes(filtered, sort);
  });

  async function handleSearch() {
    searching = true;
    try {
      await notesStore.search(searchQuery);
    } catch {}
    finally { searching = false; }
  }

  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  function onSearchInput() {
    if (isTrash) return;
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      if (searchQuery.trim().length >= 2 || notesStore.searchQuery) void handleSearch();
    }, 350);
  }

  $effect(() => {
    const id = notesStore.openRequestId;
    if (!id) return;
    notesStore.openRequestId = null;
    void handleFilterChange({ type: 'all' }).then(() => notesStore.openNote(id));
  });

  // Requests from the command palette; `insertLink` is consumed by the editor
  let searchInputEl: HTMLInputElement | null = $state(null);
  $effect(() => {
    const req = notesStore.uiRequest;
    if (!req || req.kind === 'insertLink') return;
    notesStore.uiRequest = null;
    if (req.kind === 'create') showCreate = true;
    else if (req.kind === 'filter') void handleFilterChange(req.filter);
    else if (req.kind === 'search') {
      sidebarVisible = true;
      void tick().then(() => searchInputEl?.focus());
    }
  });

  // Note opened from outside the current list stays open. A note that drops out
  // because the filter or search changed is closed.
  let trackedNoteId: string | null = null;
  let openedOutsideList = false;

  $effect(() => {
    const id = notesStore.activeNoteId;
    const inList = !!id && displayList.some((n) => n.id === id);
    if (id !== trackedNoteId) {
      trackedNoteId = id;
      openedOutsideList = !!id && !inList;
      return;
    }
    if (!id || inList) {
      openedOutsideList = false;
      return;
    }
    if (openedOutsideList) return;
    // What was typed in it is saved on the way out.
    untrack(() => notesStore.leaveNote());
  });

  async function handleFilterChange(f: ActiveFilter) {
    activeFilter = f;
    searchQuery = '';
    // Smart views are applied by the effect below (also re-applied when edited)
    if (f.type !== 'smart') await notesStore.setFilter(toNoteFilter(f));
    if (f.type === 'trash') await notesStore.refreshTrash();
  }

  $effect(() => {
    if (activeFilter.type !== 'smart') return;
    const view = notesStore.smartViews.find((v) => v.id === activeFilter.id);
    if (!view) {
      void handleFilterChange({ type: 'all' });
      return;
    }
    void notesStore.setFilter(toNoteFilter(activeFilter, notesStore.smartViews));
  });

  async function createNote() {
    if (!createTitle.trim()) return;

    const input: NoteCreateInput = {
      title: createTitle.trim(),
      bindings: contextBindings(activeFilter),
      tag_names: activeFilter.type === 'tag' && activeFilter.id ? [activeFilter.id] : undefined,
      template_id: createTemplate || undefined,
    };
    try {
      const note = await notesStore.createNote(input);

      if (activeFilter.type === 'folder' && activeFilter.id) {
        await api.notes.noteAddFolder(note.id, activeFilter.id);
        await notesStore.refresh();
      }

      showCreate = false;
      createTitle = '';
      await notesStore.openNote(note.id);
    } catch {}
  }

  async function handleSelectNote(id: string) {
    await notesStore.openNote(id);
  }

  // Without the browser module the names come from the labels, and a reference without one is shown by its kind and a short id (10.3).
  function workspaceName(id: string): string {
    return directory.name('workspace', id, $t);
  }

  function workspaceColor(id: string): string | undefined {
    return directory.color('workspace', id);
  }

  function profileName(id: string): string {
    return directory.name('profile', id, $t);
  }

  function folderName(id: string): string {
    return notesStore.folders.find((f) => f.id === id)?.name ?? id;
  }

  function folderDescendantIds(folderId: string): Set<string> {
    const result = new Set<string>([folderId]);
    for (const f of notesStore.folders) {
      if (f.parent_id === folderId) {
        for (const id of folderDescendantIds(f.id)) result.add(id);
      }
    }
    return result;
  }

  function folderColor(id: string): string | undefined {
    return notesStore.folders.find((f) => f.id === id)?.color || undefined;
  }

  /** The heading of the list: what the sidebar selected, or the search. */
  const listTitle = $derived(
    notesListTitle(activeFilter, searchQuery, { t: $t, folders: notesStore.folders, smartViews: notesStore.smartViews }),
  );

  /** An empty list says why: nothing matches, or the view is empty; only an empty vault offers New note. */
  const emptyText = $derived.by(() => {
    if (searchQuery.trim()) return $t('common_nothing_found');
    switch (activeFilter.type) {
      case 'all': return $t('notes_list_empty');
      case 'trash': return $t('notes_list_empty_trash');
      case 'pinned': return $t('notes_list_empty_pinned');
      case 'archived': return $t('notes_list_empty_archived');
      default: return $t('common_nothing_found');
    }
  });
  const emptyCanCreate = $derived(!searchQuery.trim() && activeFilter.type === 'all');

  // ── Export / import ──────────────────────────────────────────────────────────
  let transferMode = $state<TransferMode | null>(null);
  let exportIds = $state<string[]>([]);

  function openExport(ids: string[]) {
    exportIds = ids;
    transferMode = 'export';
  }

  function exportFolder(folderId: string) {
    const ids = folderDescendantIds(folderId);
    openExport(notesStore.list.filter((n) => n.folder_ids.some((f) => ids.has(f))).map((n) => n.id));
  }

  // Imported notes inherit the active workspace/profile context
  const importBindings = $derived(contextBindings(activeFilter));
</script>

<NoteTransferDialog mode={transferMode} {exportIds} {importBindings} onclose={() => (transferMode = null)} />

<div class="page page--fill notes-page">
  <!-- Body: 3 columns full height per redesign (search lives in the sidebar) -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="page-body" bind:this={pageBodyEl} class:is-dragging={dragging !== null}>
    {#if sidebarVisible}
      <div class="on-glass sidebar" style="width: {colWidths.sidebar}%">
        <div class="search-wrap">
          <Icon name="search" size={15} />
          <input
            type="text"
            bind:this={searchInputEl}
            bind:value={searchQuery}
            oninput={onSearchInput}
            placeholder={$t('notes_search_placeholder')}
            class="search-input"
          />
          {#if searching}
            <span class="search-spinner"><Icon name="loader" size={12} /></span>
          {/if}
        </div>
        <NoteFilters
          notes={notesStore.list}
          trashCount={notesStore.trash.length}
          allTags={notesStore.allTags}
          folders={notesStore.folders}
          {activeFilter}
          onfilter={handleFilterChange}
          onexportfolder={exportFolder}
        />
        <!-- Space on Windows and macOS has no title bar of its own to carry the notes window tool -->
        <NotesSidebarFooter
          ontoggle={() => { sidebarVisible = false; }}
          onimport={() => { exportIds = []; transferMode = 'import'; }}
          onexport={() => openExport(displayList.map((n) => n.id))}
          windowItem={!standaloneNotes}
        />
      </div>
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div
        class="col-resizer"
        class:active={hoveredResizer === 'sidebar' || dragging === 'sidebar'}
        onmousedown={(e) => startColDrag(e, 'sidebar')}
        onmouseenter={() => (hoveredResizer = 'sidebar')}
        onmouseleave={() => (hoveredResizer = null)}
      ></div>
    {/if}

    <div class="on-glass list-col" style="width: {viewMode === 'table' ? colWidths.table : colWidths.list}%">
      <div class="list-header">
        <div class="list-head-row">
          <div class="list-title-group">
            <h2 class="list-title" title={listTitle}>{listTitle}</h2>
            <p class="list-sub">
              {$t(countKey('panel_notes_count', displayList.length, $locale), { n: String(displayList.length) })}
            </p>
          </div>
          <div class="list-head-actions">
            {#if !sidebarVisible}
              <button class="icon-btn" onclick={() => { sidebarVisible = true; }} title={$t('notes_btn_show_sidebar')} aria-label={$t('notes_btn_show_sidebar')}>
                <Icon name="sidebar" size={18} />
              </button>
            {/if}
            {#if isTrash}
              <ListBulkActions {isTrash} notes={displayList} />
            {:else}
              <button class="icon-btn create" onclick={() => (showCreate = true)} title={$t('notes_btn_new')} aria-label={$t('notes_btn_new')}>
                <Icon name="plus" size={18} />
              </button>
            {/if}
          </div>
        </div>
        <div class="list-actions">
          <!-- Looks like an input; the palette's own field takes focus once open -->
          <button class="palette-field" onclick={() => paletteStore.show()} title={$t('cmd_title')}>
            <Icon name="command" size={18} />
            <span class="palette-placeholder">{$t('cmd_placeholder')}</span>
            <kbd>Ctrl+P</kbd>
          </button>
          <div class="sort-ctl" title={$t('notes_sort')}>
            <div class="sort-select">
              <CustomSelect
                options={SORT_KEYS.map((key) => ({ value: key, label: $t(`notes_sort_${key}`) }))}
                value={sort.key}
                anchor=".sort-ctl"
                onchange={(v) => setSort({ ...sort, key: (v ?? 'updated_at') as SortKey })}
              />
            </div>
            <button
              class="icon-btn"
              onclick={() => setSort({ ...sort, asc: !sort.asc })}
              title={sort.asc ? $t('notes_sort_asc') : $t('notes_sort_desc')}
            >
              <Icon name={sort.asc ? 'arrow-up' : 'arrow-down'} size={18} />
            </button>
          </div>
          <button
            class="icon-btn"
            onclick={() => setViewMode(viewMode === 'list' ? 'table' : 'list')}
            title={viewMode === 'list' ? $t('notes_view_table') : $t('notes_view_list')}
          >
            <Icon name={viewMode === 'list' ? 'table' : 'layout-list'} size={18} />
          </button>
        </div>
      </div>
      <div class="list-scroll" class:table-view={viewMode === 'table'}>
        {#if viewMode === 'table'}
          <NotesTable
            notes={displayList}
            allNotes={notesStore.list}
            folders={notesStore.folders}
            {sort}
            onsort={setSort}
            contextOf={noteContextEntities}
            activeId={notesStore.activeNoteId}
            onselect={handleSelectNote}
            {emptyText}
          />
        {:else}
        <NotesList
          notes={displayList}
          activeId={notesStore.activeNoteId}
          onselect={handleSelectNote}
          oncreate={emptyCanCreate ? () => (showCreate = true) : undefined}
          {emptyText}
          search={searchQuery}
          {workspaceName}
          {workspaceColor}
          {profileName}
          {folderName}
          {folderColor}
        />
        {/if}
      </div>
    </div>
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="col-resizer"
      class:active={hoveredResizer === 'list' || dragging === 'list'}
      onmousedown={(e) => startColDrag(e, 'list')}
      onmouseenter={() => (hoveredResizer = 'list')}
      onmouseleave={() => (hoveredResizer = null)}
    ></div>

    <div class="editor-col">
      <NoteEditor allTags={notesStore.allTags} folders={notesStore.folders} />
    </div>
  </div>
</div>

<!-- Create modal -->
<!-- The core dialog: it stays inside the window's frame, below the title bar. -->
<Dialog open={showCreate && !appLock.locked} title={$t('notes_create_title')} width="360px" onclose={() => (showCreate = false)}>
  <div class="create-body">
    <!-- svelte-ignore a11y_autofocus -->
    <input
      type="text"
      bind:value={createTitle}
      placeholder={$t('notes_title_placeholder')}
      autofocus
      onkeydown={(e) => e.key === 'Enter' && createNote()}
    />
    {#if hasTemplates}
      <TemplateSelect bind:value={createTemplate} />
    {/if}
  </div>
  {#snippet footer()}
    <button class="btn btn-ghost" onclick={() => (showCreate = false)}>{$t('common_cancel')}</button>
    <button class="btn btn-primary" onclick={createNote} disabled={!createTitle.trim()}>
      {$t('common_create')}
    </button>
  {/snippet}
</Dialog>

<style>
  .notes-page {
    overflow: hidden;
    gap: 0.875rem;
    max-width: none;
  }

  /* Search now lives at the top of the sidebar (per redesign) */
  .search-wrap {
    position: relative;
    display: flex;
    align-items: center;
    margin: var(--sp-4) var(--sp-3) var(--sp-2);
    flex-shrink: 0;
  }

  .search-wrap :global(svg) {
    position: absolute;
    left: 0.75rem;
    color: var(--text-3);
    pointer-events: none;
  }

  .search-input {
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-pill);
    width: 100%;
    padding: 0 1.75rem 0 var(--sp-8);
    font-size: var(--fs-base);
    height: var(--control-h);
    outline: none;
    transition: border-color 0.15s;
  }

  .search-input:focus { border-color: var(--accent-border); }

  .search-spinner {
    position: absolute;
    right: 0.5rem;
    color: var(--accent-text);
    display: flex;
    animation: spin 1s linear infinite;
  }

  /* Three floating panes; the resize handles are the 12px gaps between them. */
  .page-body {
    flex: 1;
    display: flex;
    min-height: 0;
  }

  .page-body.is-dragging {
    user-select: none;
    cursor: col-resize;
  }

  .col-resizer {
    width: var(--sp-3);
    flex-shrink: 0;
    cursor: col-resize;
    position: relative;
    background: transparent;
  }

  .col-resizer::before {
    content: '';
    position: absolute;
    top: 0;
    bottom: 0;
    left: 50%;
    width: 1px;
    transform: translateX(-50%);
    background: transparent;
    transition: background 0.15s, width 0.1s;
  }
  .col-resizer:hover::before { background: var(--border-2); }

  .col-resizer.active::before {
    background: var(--accent);
    width: 2px;
  }

  .sidebar {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    min-width: 200px;
    border-radius: var(--radius-pane);
    background: var(--pane);
    border: 1px solid var(--pane-border);
    box-shadow: var(--pane-highlight), var(--pane-shadow);
    -webkit-backdrop-filter: var(--pane-filter);
    backdrop-filter: var(--pane-filter);
  }

  /* The list or table gives way before the note does (.editor-col). */
  .list-col {
    flex-shrink: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 280px;
    container-type: inline-size;
    border-radius: var(--radius-pane);
    background: var(--pane);
    border: 1px solid var(--pane-border);
    box-shadow: var(--pane-highlight), var(--pane-shadow);
    -webkit-backdrop-filter: var(--pane-filter);
    backdrop-filter: var(--pane-filter);
  }

  /* Always two rows: the title with its buttons (back the sidebar, create or the trash's), then the
     field, the sort and the view on one line that never wraps. */
  .list-header {
    padding: var(--sp-5) var(--sp-4) var(--sp-3);
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    flex-shrink: 0;
  }
  .list-head-row { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; }
  .list-head-actions { display: flex; align-items: center; gap: 6px; flex-shrink: 0; margin-left: auto; }
  .list-head-actions .icon-btn { width: 36px; height: 36px; }

  .list-title-group { display: flex; flex-direction: column; gap: 3px; min-width: 0; flex: 1 1 auto; }
  .list-title {
    font-size: var(--fs-xl);
    font-weight: var(--fw-bold);
    letter-spacing: var(--ls-tight);
    line-height: var(--lh-tight);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .list-sub { font-size: var(--fs-xs); color: var(--text-2); }

  /* The palette field takes the rest; the controls after it keep their size */
  .list-actions { display: flex; align-items: center; gap: 6px; min-width: 0; }
  /* One group like .palette-field: its fill and frame, 36 high; the select and the arrow inside are
     bare, a line between them. Focus inside and the open list frame the whole group; the keyboard rings it. */
  .sort-ctl {
    display: flex; align-items: center; flex-shrink: 0; height: 36px;
    border: 1px solid var(--border); border-radius: var(--radius); background: var(--surface-3);
    transition: border-color var(--dur-fast), box-shadow var(--dur-fast);
  }
  .sort-ctl:hover { border-color: var(--border-2); }
  .sort-ctl:focus-within, .sort-ctl:has(:global(.trigger.open)) { border-color: var(--accent-border); }
  .sort-ctl:has(:global(:focus-visible)):not(:has(:global(.trigger.open))) { box-shadow: 0 0 0 3px var(--accent-bg); }
  .list-actions .sort-ctl .icon-btn {
    position: relative; width: 36px; height: 34px;
    border: 0; border-radius: 0 calc(var(--radius) - 1px) calc(var(--radius) - 1px) 0; background: transparent;
  }
  .list-actions .sort-ctl .icon-btn::before {
    content: ''; position: absolute; left: 0; top: 8px; bottom: 8px;
    border-left: 1px solid var(--border);
  }
  .list-actions .sort-ctl .icon-btn:focus-visible { outline: none; box-shadow: none; }
  .sort-select { width: auto; }
  .sort-select :global(.trigger) {
    height: 34px; border: 0; border-radius: 0;
    background: transparent; color: var(--text); font-size: var(--fs-base);
  }
  .sort-select :global(.trigger:focus-visible), .sort-select :global(.trigger.open) { box-shadow: none; }
  .palette-field {
    flex: 1 1 120px;
    min-width: 0;
    height: 36px;
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0 0.6rem;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-3);
    color: var(--text-3);
    font-size: var(--fs-sm);
    text-align: left;
    cursor: text;
  }
  .palette-field:hover { border-color: var(--border-2); color: var(--text-2); }
  .palette-placeholder { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .palette-field kbd {
    font-size: var(--fs-xs); color: var(--text-3);
    border: 1px solid var(--border); border-radius: var(--radius-xs); padding: 0 0.3rem; flex-shrink: 0;
  }
  .list-actions .icon-btn { width: 36px; height: 36px; }

  /* Narrow list column: the field shrinks to an icon */
  @container (max-width: 500px) {
    .palette-field kbd { display: none; }
  }
  @container (max-width: 420px) {
    .palette-placeholder { display: none; }
    .palette-field { flex: 0 0 36px; width: 36px; justify-content: center; padding: 0; }
  }

  .list-scroll {
    flex: 1;
    overflow-y: auto;
    padding: var(--sp-2);
  }
  /* The table scrolls itself, down and sideways, under its pinned head: the column only gives it the room left
     under the list's header, so the sideways bar is at the table's bottom on the screen. */
  .list-scroll.table-view {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }

  /* A note keeps room to be read and edited next to the wide table. */
  .editor-col {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 360px;
    border-radius: var(--radius-pane);
    background: var(--pane);
    border: 1px solid var(--pane-border);
    box-shadow: var(--pane-highlight), var(--pane-shadow);
    -webkit-backdrop-filter: var(--pane-filter);
    backdrop-filter: var(--pane-filter);
  }

  /* New note dialog */
  .create-body {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }
</style>

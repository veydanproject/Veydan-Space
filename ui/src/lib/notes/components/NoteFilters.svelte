<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { NoteListItem, NoteTag, NoteFolder, NoteSmartView } from '$lib/notes/types';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import SmartViewDialog from './SmartViewDialog.svelte';
  import { directory } from '$lib/core/directory';
  import { notesStore } from '$lib/notes/store/notes.svelte';
  import { compareLabel } from '$lib/core/label-order';
  import { t } from '$lib/core/i18n';
  import { findTag } from '$lib/notes/tag-name';

  const TAG_COLORS = [
    '#8b7bff', '#60a5fa', '#2dd4bf', '#f472b6',
    '#f5c451', '#34d399', '#f26d6d', '#f97316',
  ];

  interface Props {
    notes: NoteListItem[];
    trashCount?: number;
    allTags: NoteTag[];
    folders: NoteFolder[];
    activeFilter: { type: string; id?: string };
    onfilter: (filter: { type: string; id?: string }) => void;
    onexportfolder?: (folderId: string) => void;
  }

  let { notes, trashCount = 0, allTags, folders, activeFilter, onfilter, onexportfolder }: Props = $props();

  // ── One collapsed state (keys: 'ws:id', 'folder:id', 'tag:path') ──────────────
  let collapsed = $state<Set<string>>(new Set());

  function toggle(key: string) {
    const next = new Set(collapsed);
    if (next.has(key)) next.delete(key); else next.add(key);
    collapsed = next;
  }

  function keysFolded(keys: string[]): boolean {
    return keys.length > 0 && keys.every((key) => collapsed.has(key));
  }

  function toggleKeys(keys: string[]) {
    const fold = keysFolded(keys);
    const next = new Set(collapsed);
    for (const key of keys) {
      if (fold) next.delete(key);
      else next.add(key);
    }
    collapsed = next;
  }

  function walkFolders(nodes: FolderNode[], keys: string[]) {
    for (const node of nodes) {
      if (node.children.length > 0) keys.push(`folder:${node.folder.id}`);
      walkFolders(node.children, keys);
    }
  }

  function walkTags(nodes: TagNode[], keys: string[]) {
    for (const node of nodes) {
      if (node.children.length > 0) keys.push(`tag:${node.fullPath}`);
      walkTags(node.children, keys);
    }
  }

  // ── Tag popup ─────────────────────────────────────────────────────────────────
  let popupOpen    = $state(false);
  let popupPrefix  = $state('');
  let popupInput   = $state('');
  let popupColor   = $state(TAG_COLORS[0]);
  let popupInputEl = $state<HTMLInputElement | null>(null);
  let creating     = $state(false);

  function openPopup(prefix = '') {
    popupPrefix = prefix;
    popupInput  = prefix;
    popupColor  = TAG_COLORS[0];
    popupOpen   = true;
    setTimeout(() => popupInputEl?.focus(), 50);
  }

  function closePopup() {
    popupOpen   = false;
    popupInput  = '';
    popupPrefix = '';
  }

  // "Работа" typed when "работа" exists is that tag, not a new one (tag-name.ts).
  const popupIsNew = $derived(
    popupInput.trim().length > 0 &&
    !findTag(allTags, popupInput)
  );

  const popupSuggestions = $derived(
    allTags
      .filter(t => t.name.toLowerCase().includes(popupInput.toLowerCase()) && popupInput.trim().length > 0)
      .slice(0, 6)
  );

  async function createTag() {
    const name = popupInput.trim();
    if (!name || creating) return;
    // Enter on a name that exists in any case filters by that tag and keeps its color.
    const existing = findTag(allTags, name);
    if (existing) {
      closePopup();
      onfilter({ type: 'tag', id: existing.name });
      return;
    }
    creating = true;
    try {
      const tag = await notesStore.createTag(name, popupColor);
      closePopup();
      onfilter({ type: 'tag', id: tag.name });
    } finally {
      creating = false;
    }
  }

  function onPopupKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && popupInput.trim()) { e.preventDefault(); void createTag(); }
    if (e.key === 'Escape') closePopup();
  }

  // ── Tag inline edit ───────────────────────────────────────────────────────────
  let tagEditModalOpen = $state(false);
  let tagEditTag       = $state<NoteTag | null>(null);
  let tagEditName      = $state('');
  let tagEditColor     = $state(TAG_COLORS[0]);
  let tagEditInputEl   = $state<HTMLInputElement | null>(null);
  let tagEditSaving    = $state(false);

  function startEdit(tag: NoteTag, e: MouseEvent) {
    e.stopPropagation();
    tagEditTag       = tag;
    tagEditName      = tag.name;
    tagEditColor     = tag.color;
    tagEditModalOpen = true;
    setTimeout(() => tagEditInputEl?.focus(), 50);
  }

  function closeTagEditModal() {
    tagEditModalOpen = false;
    tagEditTag       = null;
    tagEditName      = '';
  }

  async function saveTagEdit() {
    if (!tagEditTag || !tagEditName.trim() || tagEditSaving) return;
    tagEditSaving = true;
    try {
      await notesStore.updateTag(tagEditTag.id, tagEditName.trim(), tagEditColor);
      closeTagEditModal();
    } finally {
      tagEditSaving = false;
    }
  }

  function onTagEditKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && tagEditName.trim()) { e.preventDefault(); void saveTagEdit(); }
    if (e.key === 'Escape') closeTagEditModal();
  }

  async function deleteTag(tag: NoteTag, e: MouseEvent) {
    e.stopPropagation();
    await notesStore.deleteTag(tag.id);
    if (activeFilter.type === 'tag' && activeFilter.id === tag.name) onfilter({ type: 'all' });
  }

  // ── Folder modal ──────────────────────────────────────────────────────────────
  let folderModalOpen     = $state(false);
  let folderModalParentId = $state('');
  let folderInput         = $state('');
  let folderColor         = $state(TAG_COLORS[0]);
  let folderInputEl       = $state<HTMLInputElement | null>(null);
  let creatingFolder      = $state(false);

  function openFolderModal(parentId = '') {
    folderModalParentId = parentId;
    folderInput         = '';
    folderColor         = TAG_COLORS[0];
    folderModalOpen     = true;
    setTimeout(() => folderInputEl?.focus(), 50);
  }

  function closeFolderModal() {
    folderModalOpen     = false;
    folderInput         = '';
    folderModalParentId = '';
  }

  async function createFolder() {
    if (!folderInput.trim() || creatingFolder) return;
    creatingFolder = true;
    try {
      const f = await notesStore.createFolder(folderInput.trim(), folderModalParentId || undefined, folderColor);
      closeFolderModal();
      onfilter({ type: 'folder', id: f.id });
    } finally {
      creatingFolder = false;
    }
  }

  function onFolderModalKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && folderInput.trim()) { e.preventDefault(); void createFolder(); }
    if (e.key === 'Escape') closeFolderModal();
  }

  // ── Folder context menu ───────────────────────────────────────────────────────
  let openMenuKey      = $state<string | null>(null); // 'folder:id' | 'tag:path'
  // The menu is position: fixed (the overflow-y of the .filters scroller would clip it)
  let menuPos          = $state({ x: 0, y: 0 });
  function openMenuAt(e: MouseEvent, key: string) {
    e.stopPropagation();
    if (openMenuKey === key) { openMenuKey = null; return; }
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menuPos = { x: r.right, y: r.bottom + 4 };
    openMenuKey = key;
  }
  let hoveredFolderId  = $state<string | null>(null);
  let hoveredTagPath   = $state<string | null>(null);

  const openFolderMenuId = $derived(openMenuKey?.startsWith('folder:') ? openMenuKey.slice('folder:'.length) : null);
  const openTagMenuPath  = $derived(openMenuKey?.startsWith('tag:')    ? openMenuKey.slice('tag:'.length)    : null);

  // ── Folder inline edit ────────────────────────────────────────────────────────
  let editFolderId    = $state<string | null>(null);
  let editFolderName  = $state('');
  let editFolderColor = $state('');

  function startEditFolder(f: NoteFolder, e: MouseEvent) {
    e.stopPropagation();
    editFolderId    = f.id;
    editFolderName  = f.name;
    editFolderColor = f.color;
  }

  async function saveEditFolder(e?: KeyboardEvent | MouseEvent) {
    if (e instanceof KeyboardEvent && e.key !== 'Enter') return;
    if (!editFolderId || !editFolderName.trim()) return;
    await notesStore.updateFolder(editFolderId, editFolderName.trim(), editFolderColor);
    editFolderId = null;
  }

  async function deleteFolder(f: NoteFolder, e: MouseEvent) {
    e.stopPropagation();
    await notesStore.deleteFolder(f.id);
    if (activeFilter.type === 'folder' && activeFilter.id === f.id) onfilter({ type: 'all' });
  }

  // ── Folder tree ───────────────────────────────────────────────────────────────
  interface FolderNode {
    folder: NoteFolder;
    children: FolderNode[];
    noteCount: number;
  }

  const folderTree = $derived.by((): FolderNode[] => {
    const nodeMap = new Map<string, FolderNode>();
    for (const f of folders) nodeMap.set(f.id, { folder: f, children: [], noteCount: 0 });
    // Archived notes are not counted, as everywhere else in the sidebar and in the phone's note_nav
    for (const note of notes) {
      if (note.archived) continue;
      for (const fid of note.folder_ids) {
        if (nodeMap.has(fid)) nodeMap.get(fid)!.noteCount++;
      }
    }
    const roots: FolderNode[] = [];
    for (const f of folders) {
      const node = nodeMap.get(f.id)!;
      if (f.parent_id && nodeMap.has(f.parent_id)) nodeMap.get(f.parent_id)!.children.push(node);
      else roots.push(node);
    }
    roots.sort((a, b) => compareLabel(a.folder.name, b.folder.name));
    for (const node of nodeMap.values()) node.children.sort((a, b) => compareLabel(a.folder.name, b.folder.name));

    // Propagate child counts to parents (post-order)
    function sumChildren(node: FolderNode): number {
      const childTotal = node.children.reduce((s, c) => s + sumChildren(c), 0);
      node.noteCount += childTotal;
      return node.noteCount;
    }
    roots.forEach(sumChildren);

    return roots;
  });

  // ── Tag tree ──────────────────────────────────────────────────────────────────
  interface TagNode {
    label: string;
    fullPath: string;
    tag: NoteTag | null;
    children: TagNode[];
  }

  const tagTree = $derived.by((): TagNode[] => {
    const tagByName = new Map<string, NoteTag>(allTags.map(t => [t.name, t]));
    const nodeMap   = new Map<string, TagNode>();

    function getOrCreate(path: string): TagNode {
      if (nodeMap.has(path)) return nodeMap.get(path)!;
      const parts = path.split('/');
      const node: TagNode = { label: parts[parts.length - 1], fullPath: path, tag: tagByName.get(path) ?? null, children: [] };
      nodeMap.set(path, node);
      return node;
    }

    const roots = new Map<string, TagNode>();
    for (const tag of allTags) {
      const parts = tag.name.split('/');
      if (parts.length === 1) {
        if (!roots.has(tag.name)) roots.set(tag.name, getOrCreate(tag.name));
      } else {
        const rootKey = parts[0];
        if (!roots.has(rootKey)) roots.set(rootKey, getOrCreate(rootKey));
        for (let i = 1; i < parts.length; i++) {
          const parent = getOrCreate(parts.slice(0, i).join('/'));
          const child  = getOrCreate(parts.slice(0, i + 1).join('/'));
          if (!parent.children.some(c => c.fullPath === child.fullPath)) parent.children.push(child);
        }
      }
    }
    const sortTree = (nodes: TagNode[]) => {
      nodes.sort((a, b) => compareLabel(a.label, b.label));
      for (const node of nodes) sortTree(node.children);
    };
    const sorted = [...roots.values()];
    sortTree(sorted);
    return sorted;
  });

  // ── Workspace tree ────────────────────────────────────────────────────────────
  const workspaceTree = $derived.by(() =>
    directory.list('workspace').map(ws => {
      const wsTag   = `workspace:${ws.id}`;
      const wsNotes = notes.filter(n =>
        n.bindings.includes(wsTag) && !n.bindings.some(b => b.startsWith('profile:')) && !n.archived
      );
      const profiles = directory.list('profile').filter((p) => p.parent === wsTag)
        .map(p => ({ ...p, noteCount: notes.filter(n => n.bindings.includes(`profile:${p.id}`) && !n.archived).length }))
        .sort((a, b) => compareLabel(a.name, b.name));
      return { ...ws, noteCount: wsNotes.length, profiles };
    }).filter(ws => ws.noteCount > 0 || ws.profiles.length > 0)
      .sort((a, b) => compareLabel(a.name, b.name))
  );

  // ── Smart views ───────────────────────────────────────────────────────────────
  let smartDialogOpen = $state(false);
  let smartEditing = $state<NoteSmartView | null>(null);
  let hoveredSmartId = $state<string | null>(null);

  function openSmartDialog(view: NoteSmartView | null, e?: MouseEvent) {
    e?.stopPropagation();
    smartEditing = view;
    smartDialogOpen = true;
    openMenuKey = null;
  }

  async function deleteSmartView(view: NoteSmartView, e: MouseEvent) {
    e.stopPropagation();
    openMenuKey = null;
    await notesStore.deleteSmartView(view.id);
    if (activeFilter.type === 'smart' && activeFilter.id === view.id) onfilter({ type: 'all' });
  }

  // ── Sites (domain: bindings from browser capture) ─────────────────────────────
  const siteList = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const n of notes) {
      if (n.archived) continue;
      for (const b of n.bindings) {
        if (b.startsWith('domain:')) {
          const d = b.slice('domain:'.length);
          counts.set(d, (counts.get(d) ?? 0) + 1);
        }
      }
    }
    return [...counts.entries()]
      .map(([domain, noteCount]) => ({ domain, noteCount }))
      .sort((a, b) => b.noteCount - a.noteCount || a.domain.localeCompare(b.domain));
  });

  // ── Helpers ───────────────────────────────────────────────────────────────────
  function isGlobal(bindings: string[]) {
    return !bindings.some(b => b.startsWith('workspace:') || b.startsWith('profile:'));
  }

  const countAll      = $derived(notes.filter(n => !n.archived).length);
  const countGlobal   = $derived(notes.filter(n => isGlobal(n.bindings) && !n.archived).length);
  const countPinned   = $derived(notes.filter(n => n.pinned && !n.archived).length);
  const countArchived = $derived(notes.filter(n => n.archived).length);

  const tagCountMap = $derived.by((): Map<string, number> => {
    const m = new Map<string, number>();
    for (const note of notes) {
      if (note.archived) continue;
      for (const tag of note.tags) m.set(tag.name, (m.get(tag.name) ?? 0) + 1);
    }
    return m;
  });

  function nodeCount(path: string, hasChildren: boolean): number {
    if (!hasChildren) return tagCountMap.get(path) ?? 0;
    let total = 0;
    for (const [name, c] of tagCountMap) {
      if (name === path || name.startsWith(path + '/')) total += c;
    }
    return total;
  }

  function filterClass(type: string, id?: string) {
    return activeFilter.type === type && activeFilter.id === id ? 'nav-item active' : 'nav-item';
  }
</script>

<svelte:window
  onkeydown={(e) => { if (e.key === 'Escape') { closePopup(); openMenuKey = null; } }}
  onclick={() => { openMenuKey = null; }}
/>

{#snippet folderNode(node: FolderNode, depth: number)}
  {@const f = node.folder}
  {@const isCollapsed = collapsed.has('folder:' + f.id)}

  <div class="tree-node">
    {#if editFolderId === f.id}
      <div class="edit-row" style="padding-left: {depth > 0 ? '1.5rem' : '0.5rem'}">
        <input type="color" bind:value={editFolderColor} class="color-pick" />
        <input
          type="text"
          bind:value={editFolderName}
          class="edit-input"
          placeholder={f.name}
          onkeydown={(e) => { if (e.key === 'Enter') saveEditFolder(e); if (e.key === 'Escape') editFolderId = null; }}
        />
        <button class="btn-icon-xs btn-save" onclick={saveEditFolder} disabled={!editFolderName.trim()}>
          <Icon name="check" size={11} />
        </button>
        <button class="btn-icon-xs btn-cancel" onclick={() => editFolderId = null}>
          <Icon name="x" size={11} />
        </button>
      </div>
    {:else}
      <div class="tree-row folder-row" role="group"
        onmouseenter={() => hoveredFolderId = f.id}
        onmouseleave={() => hoveredFolderId = null}
      >
        <button
          class="{filterClass('folder', f.id)} tree-item"
          style="padding-left: {depth > 0 ? '1.5rem' : '0.5rem'}"
          onclick={() => onfilter({ type: 'folder', id: f.id })}
        >
          {#if depth === 0}
            <span class="dot-slot"><span class="dot" style="background:{f.color}"></span></span>
          {:else}
            <span class="icon-slot"><span class="dot" style="background:{f.color}"></span></span>
          {/if}
          <span class="item-name">{f.name}</span>
        </button>

        {#if node.children.length > 0}
          <button class="collapse-trigger" onclick={(e) => { e.stopPropagation(); toggle('folder:' + f.id); }}>
            <Icon name={isCollapsed ? 'chevron-right' : 'chevron-down'} size={10} />
          </button>
        {/if}

        <div class="count-wrap"
          class:menu-open={openFolderMenuId === f.id}
          class:hovered={hoveredFolderId === f.id}
          role="presentation"
          onclick={(e) => e.stopPropagation()}
        >
          {#if node.noteCount > 0}
            <span class="count count-num">{node.noteCount}</span>
          {/if}
          <button
            class="dots-btn"
            onclick={(e) => openMenuAt(e, `folder:${f.id}`)}
            title={$t('notes_item_actions')}
          >
            <Icon name="more-vertical" size={11} />
          </button>
          {#if openFolderMenuId === f.id}
            <div class="folder-menu" style="left:{menuPos.x}px; top:{menuPos.y}px">
              {#if !f.parent_id}
                <button onclick={() => { openFolderModal(f.id); openMenuKey = null; }}>
                  <Icon name="folder-plus" size={11} /><span>{$t('notes_folder_sub')}</span>
                </button>
              {/if}
              <button onclick={(e) => { startEditFolder(f, e); openMenuKey = null; }}>
                <Icon name="pencil" size={11} /><span>{$t('notes_item_edit')}</span>
              </button>
              {#if onexportfolder}
                <button onclick={() => { onexportfolder(f.id); openMenuKey = null; }}>
                  <Icon name="download" size={11} /><span>{$t('notes_folder_export')}</span>
                </button>
              {/if}
              <button class="menu-del" onclick={(e) => { void deleteFolder(f, e); openMenuKey = null; }}>
                <Icon name="trash-2" size={11} /><span>{$t('notes_tag_delete')}</span>
              </button>
            </div>
          {/if}
        </div>
      </div>

      {#if !isCollapsed && node.children.length > 0}
        {#each node.children as child (child.folder.id)}
          {@render folderNode(child, depth + 1)}
        {/each}
      {/if}
    {/if}
  </div>
{/snippet}

{#snippet tagNode(node: TagNode, depth: number)}
  {@const hasChildren = node.children.length > 0}
  {@const count       = nodeCount(node.fullPath, hasChildren)}
  {@const isCollapsed = collapsed.has('tag:' + node.fullPath)}
  {@const filterType  = hasChildren ? 'tag-group' : 'tag'}

  <div class="tree-node">
      <div class="tree-row tag-row" role="group" style="padding-left: {depth * 1.0}rem"
        onmouseenter={() => hoveredTagPath = node.fullPath}
        onmouseleave={() => hoveredTagPath = null}
      >
        <button
          class="{filterClass(filterType, node.fullPath)} tree-item"
          onclick={() => onfilter({ type: filterType, id: node.fullPath })}
        >
          {#if depth === 0}
            <span class="dot-slot"><span class="dot" style="background:{node.tag?.color ?? 'var(--text-3)'}; opacity:{node.tag ? 1 : 0.4}"></span></span>
          {:else}
            <span class="icon-slot"><span class="dot" style="background:{node.tag?.color ?? 'var(--text-3)'}; opacity:{node.tag ? 1 : 0.4}"></span></span>
          {/if}
          <span class="item-name">{node.label}</span>
        </button>

        {#if hasChildren}
          <button class="collapse-trigger" onclick={(e) => { e.stopPropagation(); toggle('tag:' + node.fullPath); }}>
            <Icon name={isCollapsed ? 'chevron-right' : 'chevron-down'} size={10} />
          </button>
        {/if}

        <div class="count-wrap"
          class:menu-open={openTagMenuPath === node.fullPath}
          class:hovered={node.tag && hoveredTagPath === node.fullPath}
          role="presentation"
          onclick={(e) => e.stopPropagation()}
        >
          {#if count > 0}
            <span class="count count-num">{count}</span>
          {/if}
          {#if node.tag}
            <button
              class="dots-btn"
              onclick={(e) => openMenuAt(e, `tag:${node.fullPath}`)}
              title={$t('notes_item_actions')}
            >
              <Icon name="more-vertical" size={11} />
            </button>
            {#if openTagMenuPath === node.fullPath}
              <div class="folder-menu" style="left:{menuPos.x}px; top:{menuPos.y}px">
                <button onclick={(e) => { startEdit(node.tag!, e); openMenuKey = null; }}>
                  <Icon name="pencil" size={11} /><span>{$t('notes_item_edit')}</span>
                </button>
                <button class="menu-del" onclick={(e) => { void deleteTag(node.tag!, e); openMenuKey = null; }}>
                  <Icon name="trash-2" size={11} /><span>{$t('notes_tag_delete')}</span>
                </button>
              </div>
            {/if}
          {/if}
        </div>
      </div>

      {#if hasChildren && !isCollapsed}
        {#each node.children as child (child.fullPath)}
          {@render tagNode(child, depth + 1)}
        {/each}
        <div class="group-add-row" style="padding-left: {(depth + 1) * 1.0 + 1.25}rem">
          <button class="btn-group-add" onclick={() => openPopup(node.fullPath + '/')}>
            <Icon name="plus" size={10} />
            <span>{$t('notes_tag_add_in', { name: node.label })}</span>
          </button>
        </div>
      {/if}
  </div>
{/snippet}

<div class="filters">
  <!-- Global filters -->
  <div class="filter-section global-section">
    <button class={filterClass('all')} onclick={() => onfilter({ type: 'all' })}>
      <span class="global-icon-slot"><Icon name="file-text" size={13} /></span>
      {$t('notes_filter_all')}
      <span class="count">{countAll}</span>
    </button>
    <!-- Global (bound to no workspace or profile) says nothing in a product without workspaces where every note is global -->
    {#if directory.owned('workspace') || workspaceTree.length > 0 || countGlobal !== countAll || activeFilter.type === 'global'}
      <button class={filterClass('global')} onclick={() => onfilter({ type: 'global' })}>
        <span class="global-icon-slot"><Icon name="globe" size={13} /></span>
        {$t('notes_filter_global')}
        <span class="count">{countGlobal}</span>
      </button>
    {/if}
    <button class={filterClass('pinned')} onclick={() => onfilter({ type: 'pinned' })}>
      <span class="global-icon-slot"><Icon name="pin" size={13} /></span>
      {$t('notes_filter_pinned')}
      <span class="count">{countPinned}</span>
    </button>
    <button class={filterClass('archived')} onclick={() => onfilter({ type: 'archived' })}>
      <span class="global-icon-slot"><Icon name="archive" size={13} /></span>
      {$t('notes_filter_archived')}
      <span class="count">{countArchived}</span>
    </button>
    <button class={filterClass('trash')} onclick={() => onfilter({ type: 'trash' })}>
      <span class="global-icon-slot"><Icon name="trash-2" size={13} /></span>
      {$t('notes_filter_trash')}
      <span class="count">{trashCount}</span>
    </button>
  </div>

  <!-- Workspaces -->
  {#if workspaceTree.length > 0}
    <div class="filter-section">
      {@render foldHead($t('notes_filter_workspaces'), 'workspaces', () => workspaceTree.filter((ws) => ws.profiles.length > 0).map((ws) => `ws:${ws.id}`))}
      {#if !collapsed.has('workspaces')}
      {#each workspaceTree as ws (ws.id)}
        <div class="tree-node">
          <div class="tree-row">
            <button class="{filterClass('workspace', ws.id)} tree-item" onclick={() => onfilter({ type: 'workspace', id: ws.id })}>
              <span class="dot-slot"><span class="dot" style="background:{ws.color}"></span></span>
              <span class="item-name">{ws.name}</span>
            </button>
            {#if ws.profiles.length > 0}
              <button class="collapse-trigger" onclick={(e) => { e.stopPropagation(); toggle('ws:' + ws.id); }}>
                <Icon name={collapsed.has('ws:' + ws.id) ? 'chevron-right' : 'chevron-down'} size={10} />
              </button>
            {/if}
            {#if ws.noteCount > 0}<span class="count">{ws.noteCount}</span>{/if}
          </div>
          {#if !collapsed.has('ws:' + ws.id)}
            {#each ws.profiles as p (p.id)}
              <button class="{filterClass('profile', p.id)} nav-item profile-item" onclick={() => onfilter({ type: 'profile', id: p.id })}>
                <span class="icon-slot"><Icon name="browser" size={11} /></span>
                <span class="item-name">{p.name}</span>
                <span class="count">{p.noteCount}</span>
              </button>
            {/each}
          {/if}
        </div>
      {/each}
      {/if}
    </div>
  {/if}

  <!-- Folders -->
  <div class="filter-section">
    <div class="section-label-row">
      <span class="section-label">{$t('notes_filter_folders')}</span>
      <span class="head-actions">
        {@render foldChevs('folders', () => { const keys: string[] = []; walkFolders(folderTree, keys); return keys; })}
        <span class="plus-slot">
          <button class="btn-add-area" onclick={() => openFolderModal()} title={$t('notes_folder_new')}>
            <Icon name="plus" size={11} />
          </button>
        </span>
      </span>
    </div>
    {#if !collapsed.has('folders')}
    {#each folderTree as node (node.folder.id)}
      {@render folderNode(node, 0)}
    {/each}
    {#if folderTree.length === 0}
      <span class="empty-hint">{$t('notes_filter_folders_empty')}</span>
    {/if}
    {/if}
  </div>

  <!-- Smart views -->
  <div class="filter-section">
    <div class="section-label-row">
      <span class="section-label">{$t('notes_smart_title')}</span>
      <span class="head-actions">
        {@render foldChevs('smart', () => [])}
        <span class="plus-slot">
          <button class="btn-add-area" onclick={() => openSmartDialog(null)} title={$t('notes_smart_new')}>
            <Icon name="plus" size={11} />
          </button>
        </span>
      </span>
    </div>
    {#if !collapsed.has('smart')}
    {#each notesStore.smartViews as view (view.id)}
      <div class="tree-node">
        <div class="tree-row folder-row" role="group"
          onmouseenter={() => hoveredSmartId = view.id}
          onmouseleave={() => hoveredSmartId = null}
        >
          <button class="{filterClass('smart', view.id)} tree-item" onclick={() => onfilter({ type: 'smart', id: view.id })}>
            <span class="dot-slot"><span class="dot" style="background:{view.color}"></span></span>
            <span class="item-name">{view.name}</span>
          </button>
          <div class="count-wrap"
            class:menu-open={openMenuKey === 'smart:' + view.id}
            class:hovered={hoveredSmartId === view.id}
            role="presentation"
            onclick={(e) => e.stopPropagation()}
          >
            <button class="dots-btn" onclick={(e) => openMenuAt(e, `smart:${view.id}`)} title={$t('notes_item_actions')}>
              <Icon name="more-vertical" size={11} />
            </button>
            {#if openMenuKey === 'smart:' + view.id}
              <div class="folder-menu" style="left:{menuPos.x}px; top:{menuPos.y}px">
                <button onclick={(e) => openSmartDialog(view, e)}>
                  <Icon name="pencil" size={11} /><span>{$t('notes_smart_edit')}</span>
                </button>
                <button class="menu-del" onclick={(e) => deleteSmartView(view, e)}>
                  <Icon name="trash-2" size={11} /><span>{$t('notes_tag_delete')}</span>
                </button>
              </div>
            {/if}
          </div>
        </div>
      </div>
    {/each}
    {#if notesStore.smartViews.length === 0}
      <span class="empty-hint">{$t('notes_smart_empty')}</span>
    {/if}
    {/if}
  </div>

  <!-- Sites -->
  {#if siteList.length > 0}
    <div class="filter-section">
      {@render foldHead($t('notes_filter_sites'), 'sites', () => [])}
      {#if !collapsed.has('sites')}
        {#each siteList as site (site.domain)}
          <button class={filterClass('domain', site.domain)} onclick={() => onfilter({ type: 'domain', id: site.domain })}>
            <span class="global-icon-slot"><Icon name="globe" size={12} /></span>
            <span class="item-name">{site.domain}</span>
            <span class="count">{site.noteCount}</span>
          </button>
        {/each}
      {/if}
    </div>
  {/if}

  <!-- Tags -->
  <div class="filter-section">
    <div class="section-label-row">
      <span class="section-label">{$t('notes_areas_title')}</span>
      <span class="head-actions">
        {@render foldChevs('tags', () => { const keys: string[] = []; walkTags(tagTree, keys); return keys; })}
        <span class="plus-slot">
          <button class="btn-add-area" onclick={() => openPopup()} title={$t('notes_areas_new')}>
            <Icon name="plus" size={11} />
          </button>
        </span>
      </span>
    </div>
    {#if !collapsed.has('tags')}
    {#each tagTree as node (node.fullPath)}
      {@render tagNode(node, 0)}
    {/each}
    {#if tagTree.length === 0}
      <span class="empty-hint">{$t('notes_areas_empty')}</span>
    {/if}
    {/if}
  </div>
</div>

<!-- The section's own chevron folds the section, the double one every nested level in it -->
{#snippet foldChevs(sectionKey: string, childKeys: () => string[])}
  {@const nested = childKeys()}
  {@const folded = keysFolded(nested)}
  <span class="ws-chevs">
    {#if nested.length > 0}
      <button
        class="collapse-trigger chev-slot"
        onclick={() => toggleKeys(nested)}
        title={sectionKey === 'workspaces'
          ? (folded ? $t('notes_ws_unfold_profiles') : $t('notes_ws_fold_profiles'))
          : (folded ? $t('notes_section_unfold_all') : $t('notes_section_fold_all'))}
      >
        <Icon name={folded ? 'chevrons-right' : 'chevrons-down'} size={12} />
      </button>
    {:else}
      <span class="chev-slot"></span>
    {/if}
    <button
      class="collapse-trigger chev-slot"
      onclick={() => toggle(sectionKey)}
      title={sectionKey === 'workspaces'
        ? (collapsed.has(sectionKey) ? $t('notes_ws_unfold') : $t('notes_ws_fold'))
        : (collapsed.has(sectionKey) ? $t('notes_section_unfold') : $t('notes_section_fold'))}
    >
      <Icon name={collapsed.has(sectionKey) ? 'chevron-right' : 'chevron-down'} size={10} />
    </button>
  </span>
{/snippet}

{#snippet foldHead(label: string, sectionKey: string, childKeys: () => string[])}
  <div class="section-label-row">
    <span class="section-label">{label}</span>
    <span class="head-actions">
      {@render foldChevs(sectionKey, childKeys)}
      <span class="plus-slot"></span>
    </span>
  </div>
{/snippet}

<SmartViewDialog open={smartDialogOpen} view={smartEditing} onclose={() => (smartDialogOpen = false)} />

<!-- New tag dialog -->
<Dialog open={popupOpen} width="300px" title={popupPrefix ? $t('notes_tag_in', { name: popupPrefix.replace(/\/$/, '') }) : $t('notes_areas_new')} onclose={closePopup}>
  <div class="nf-modal-body">
    {#if !popupPrefix}
      <p class="popup-hint">{$t('notes_tag_hint')} <code>{$t('notes_tag_hint_example')}</code></p>
    {/if}

    <input
      bind:this={popupInputEl}
      bind:value={popupInput}
      type="text"
      class="popup-input"
      placeholder={popupPrefix ? `${popupPrefix}${$t('notes_tag_name_ph')}` : $t('notes_tag_name_ph')}
      onkeydown={onPopupKeydown}
    />

    {#if popupSuggestions.length > 0}
      <div class="suggestions">
        {#each popupSuggestions as s (s.id)}
          <button class="sug-item" onmousedown={(e) => { e.preventDefault(); onfilter({ type: 'tag', id: s.name }); closePopup(); }}>
            <span class="dot" style="background:{s.color}"></span>
            {s.name}
          </button>
        {/each}
      </div>
    {/if}

    {#if popupIsNew}
      <div class="color-row">
        {#each TAG_COLORS as c}
          <button class="color-swatch" class:active={popupColor === c} style="background:{c}" aria-label={$t('notes_color', { c })} onclick={() => popupColor = c}></button>
        {/each}
        <label class="color-swatch color-swatch-custom" class:active={!TAG_COLORS.includes(popupColor)} title={$t('notes_color_custom')}>
          <input type="color" bind:value={popupColor} />
          {#if !TAG_COLORS.includes(popupColor)}
            <span class="custom-dot" style="background:{popupColor}"></span>
          {/if}
        </label>
      </div>
    {/if}
    <!-- The same full-width button as New folder; Enter still creates -->
    <button class="btn btn-primary btn-sm create-btn" onclick={createTag} disabled={creating || !popupIsNew}>
      {creating ? '…' : popupIsNew ? $t('notes_create_named', { name: popupInput.trim() }) : $t('notes_tag_add')}
    </button>
  </div>
</Dialog>

<!-- New folder dialog -->
<Dialog open={folderModalOpen} width="300px" title={$t('notes_folder_new')} onclose={closeFolderModal}>
  <div class="nf-modal-body">
    <input
      bind:this={folderInputEl}
      bind:value={folderInput}
      type="text"
      class="popup-input"
      placeholder={$t('notes_folder_name_ph')}
      onkeydown={onFolderModalKeydown}
    />

    <div class="parent-select-row">
      <span class="parent-select-label">{$t('notes_folder_parent')}</span>
      <select class="parent-select" bind:value={folderModalParentId}>
        <option value="">{$t('notes_folder_root')}</option>
        {#each folders as f (f.id)}
          <option value={f.id}>{f.name}</option>
        {/each}
      </select>
    </div>

    <div class="color-row">
      {#each TAG_COLORS as c}
        <button class="color-swatch" class:active={folderColor === c} style="background:{c}" aria-label={$t('notes_color', { c })} onclick={() => folderColor = c}></button>
      {/each}
      <label class="color-swatch color-swatch-custom" class:active={!TAG_COLORS.includes(folderColor)} title={$t('notes_color_custom')}>
        <input type="color" bind:value={folderColor} />
        {#if !TAG_COLORS.includes(folderColor)}
          <span class="custom-dot" style="background:{folderColor}"></span>
        {/if}
      </label>
    </div>

    <button class="btn btn-primary btn-sm create-btn" onclick={createFolder} disabled={!folderInput.trim() || creatingFolder}>
      {creatingFolder ? '…' : folderInput.trim() ? $t('notes_create_named', { name: folderInput.trim() }) : $t('notes_folder_create')}
    </button>
  </div>
</Dialog>

<!-- Edit tag dialog -->
<Dialog open={tagEditModalOpen} width="300px" title={$t('notes_tag_edit')} onclose={closeTagEditModal}>
  <div class="nf-modal-body">
    <input
      bind:this={tagEditInputEl}
      bind:value={tagEditName}
      type="text"
      class="popup-input"
      placeholder={$t('notes_tag_name_ph')}
      onkeydown={onTagEditKeydown}
    />

    <div class="color-row">
      {#each TAG_COLORS as c}
        <button class="color-swatch" class:active={tagEditColor === c} style="background:{c}" aria-label={$t('notes_color', { c })} onclick={() => tagEditColor = c}></button>
      {/each}
      <label class="color-swatch color-swatch-custom" class:active={!TAG_COLORS.includes(tagEditColor)} title={$t('notes_color_custom')}>
        <input type="color" bind:value={tagEditColor} />
        {#if !TAG_COLORS.includes(tagEditColor)}
          <span class="custom-dot" style="background:{tagEditColor}"></span>
        {/if}
      </label>
    </div>

    <button class="btn btn-primary btn-sm create-btn" onclick={saveTagEdit} disabled={!tagEditName.trim() || tagEditSaving}>
      {tagEditSaving ? '…' : $t('notes_btn_save')}
    </button>
  </div>
</Dialog>

<style>
  .filters {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding: var(--sp-2) 0;
    overflow-y: auto;
  }

  .filter-section {
    display: flex;
    flex-direction: column;
    padding: 0 var(--sp-2);
    gap: 0.1rem;
  }

  .filter-section + .filter-section {
    border-top: 1px solid var(--border);
    margin-top: var(--sp-1);
    padding-top: var(--sp-1);
  }

  /* Section header: one pattern for every section */
  .section-label-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.6rem var(--sp-2) 0.2rem;
  }

  .section-label {
    font-size: var(--fs-2xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-3);
    font-weight: 600;
  }

  .section-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
  }
  .head-actions { display: inline-flex; align-items: center; margin-left: auto; }
  .head-actions .ws-chevs { margin-left: 0; }
  .ws-chevs { display: inline-flex; align-items: center; margin-left: auto; }
  .chev-slot {
    width: 22px;
    height: 22px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    padding: 0;
  }
  .plus-slot {
    width: 22px;
    height: 22px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  /* Nav item: the base row of the list */
  .nav-item {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 0.55rem var(--sp-3);
    border: none;
    background: none;
    cursor: pointer;
    color: var(--text-body);
    font-size: 0.85rem;
    font-weight: var(--fw-semibold);
    border-radius: 9px;
    text-align: left;
    transition: background 0.1s, color 0.1s;
    width: 100%;
    min-width: 0;
  }

  .nav-item:hover  { background: var(--surface-2); color: var(--text); }
  .nav-item.active { background: var(--accent-bg); color: var(--accent-text); }

  .count {
    margin-left: auto;
    font-size: var(--fs-2xs);
    font-weight: var(--fw-semibold);
    color: var(--text-faint);
    background: var(--surface-2);
    border-radius: 50%;
    flex-shrink: 0;
    min-width: 1.25rem;
    height: 1.25rem;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 0.15rem;
  }
  .nav-item.active .count { background: var(--accent-bg); color: var(--accent-text); }

  /* count in a tree-row: makes up for the button padding it lacks */
  .tree-row > .count { margin-right: var(--sp-2); }

  /* Profiles: the children of a workspace */
  .profile-item { padding-left: var(--sp-6); font-size: var(--fs-sm); }

  /* Tree: one pattern for workspaces, folders and tags */
  .tree-node { display: flex; flex-direction: column; }
  .tree-row  { display: flex; align-items: center; }
  .tree-item { flex: 1; min-width: 0; }

  .tree-row:hover .item-actions { opacity: 1; max-width: 6rem; margin-right: var(--sp-1); }

  /* Fixed-width slots keep the text on one vertical line */
  .dot-slot  { flex-shrink: 0; display: flex; align-items: center; margin-left: calc(0.2rem - 5px); margin-right: calc(-0.2rem + 5px); }
  .icon-slot { flex-shrink: 0; display: flex; align-items: center; margin-left: calc(0.1rem - 5px); margin-right: calc(-0.1rem + 5px); }

  /* Icons of the global rows: as wide as dot-slot + dot + gap, so their text lines up with the workspaces */
  .global-icon-slot {
    width: calc(0.4rem + 8px);
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    margin-left: -5px;
    margin-right: calc(-0.4rem + 5px);
  }

  /* Colour dot */
  .dot {
    width: 8px; height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  /* Item name */
  .item-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  /* Collapse button */
  .collapse-trigger {
    cursor: pointer;
    color: var(--text-3);
    padding: 0 var(--sp-1);
    display: flex;
    align-items: center;
    flex-shrink: 0;
    background: none;
    border: none;
  }

  /* Actions (edit, delete), shown on hover */
  .item-actions {
    display: flex;
    align-items: center;
    gap: 0.1rem;
    opacity: 0;
    max-width: 0;
    overflow: hidden;
    transition: max-width 0.15s, opacity 0.1s;
    flex-shrink: 0;
  }

  .action-btn {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-3);
    padding: 0.15rem;
    border-radius: 3px;
    display: flex;
    align-items: center;
  }
  .action-btn:hover     { color: var(--text); background: var(--surface-2); }
  .action-del:hover     { color: var(--danger-text); }

  /* Count-wrap: the number and the actions menu of a folder */
  .count-wrap {
    position: relative;
    flex-shrink: 0;
    margin-left: auto;
    margin-right: var(--sp-2);
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 1.25rem;
    height: 1.25rem;
  }

  .count-wrap .count-num {
    margin-left: 0;
  }

  .dots-btn {
    display: none;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    background: var(--surface-2);
    border: none;
    border-radius: 50%;
    cursor: pointer;
    color: var(--text-2);
    padding: 0;
    flex-shrink: 0;
  }

  .dots-btn:hover { color: var(--text); background: var(--surface); }

  /* The dots show on hover of the row or while the menu is open */
  .count-wrap.hovered .count-num,
  .count-wrap.menu-open .count-num { display: none; }

  .count-wrap.hovered .dots-btn,
  .count-wrap.menu-open .dots-btn  { display: flex; }

  .folder-menu {
    position: fixed;
    transform: translateX(-100%);
    z-index: var(--z-popover);
    background: var(--surface-drawer);
    border: 1px solid var(--border-2);
    border-radius: var(--radius);
    padding: 4px;
    min-width: 150px;
    box-shadow: var(--shadow-lg);
  }

  .folder-menu button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.45rem 0.6rem;
    font-size: var(--fs-sm);
    font-weight: var(--fw-medium);
    background: none;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text-body);
    cursor: pointer;
    text-align: left;
  }

  .folder-menu button:hover  { background: var(--surface-2); color: var(--text); }
  .folder-menu .menu-del:hover { color: var(--danger-text); }

  /* Inline edit row */
  .edit-row {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    padding-top: 0.15rem;
    padding-bottom: 0.15rem;
    padding-right: var(--sp-2);
  }

  .color-pick {
    width: 1.4rem; height: 1.4rem;
    border: none; border-radius: 50%;
    padding: 0; cursor: pointer;
    background: none; flex-shrink: 0;
  }

  .edit-input {
    flex: 1;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: var(--sp-1) 0.4rem;
    font-size: var(--fs-sm);
    color: var(--text);
    outline: none;
    min-width: 0;
  }
  .edit-input:focus { border-color: var(--accent); }

  .btn-icon-xs {
    background: none;
    border: none;
    cursor: pointer;
    padding: 0.2rem;
    border-radius: 3px;
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }
  .btn-save            { color: var(--accent); }
  .btn-save:hover      { background: var(--accent-bg); }
  .btn-save:disabled   { opacity: 0.4; cursor: default; }
  .btn-cancel          { color: var(--text-3); }
  .btn-cancel:hover    { background: var(--surface-2); }

  /* Add button in a section header */
  .btn-add-area {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-2);
    padding: 0.1rem 0.2rem;
    border-radius: 3px;
    display: flex;
    align-items: center;
  }
  .btn-add-area:hover { color: var(--accent); background: var(--surface-2); }

  /* Add-a-subtag button inside a group */
  .group-add-row { padding-top: 0.05rem; padding-bottom: 0.1rem; }

  .btn-group-add {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-3);
    font-size: var(--fs-xs);
    padding: 0.15rem 0.3rem;
    border-radius: 3px;
  }
  .btn-group-add:hover { color: var(--accent); background: var(--accent-bg); }

  .empty-hint {
    font-size: var(--fs-xs);
    color: var(--text-2);
    padding: 0.2rem var(--sp-2);
    font-style: italic;
  }

  /* Dialogs */
  .nf-modal-body {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  .popup-hint {
    font-size: var(--fs-2xs);
    color: var(--text-3);
    line-height: 1.3;
    padding: 0 0.1rem;
  }
  .popup-hint code {
    background: var(--surface-2);
    border-radius: 3px;
    padding: 0.05rem var(--sp-1);
    font-size: var(--fs-2xs);
  }

  /* .popup-input takes the global field of base.css: the same size as every other form */

  .parent-select-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .parent-select-label {
    font-size: var(--fs-xs);
    color: var(--text-3);
    flex-shrink: 0;
  }

  /* The colour alone: the chevron is the global select's background image. */
  .parent-select {
    flex: 1;
    padding: var(--sp-1) 2rem var(--sp-1) 0.4rem;
    font-size: var(--fs-sm);
    background-color: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text);
    outline: none;
    cursor: pointer;
  }
  .parent-select:focus { border-color: var(--accent); }

  .suggestions {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }

  .sug-item {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem var(--sp-2);
    background: none;
    border: none;
    cursor: pointer;
    font-size: var(--fs-sm);
    color: var(--text);
    text-align: left;
    transition: background 0.1s;
  }
  .sug-item:hover { background: var(--surface); }

  .color-row { display: flex; gap: 0.3rem; flex-wrap: wrap; }

  .color-swatch {
    width: 18px; height: 18px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    padding: 0;
    transition: transform 0.1s;
  }
  .color-swatch.active { border-color: var(--text); transform: scale(1.2); }
  .color-swatch:hover  { transform: scale(1.15); }

  .color-swatch-custom {
    position: relative;
    background: conic-gradient(#f43f5e, #f97316, #eab308, #22c55e, #06b6d4, #6366f1, #ec4899, #f43f5e);
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .color-swatch-custom input[type="color"] {
    position: absolute; inset: 0; width: 100%; height: 100%;
    opacity: 0; cursor: pointer; border: none; padding: 0; margin: 0;
  }

  .custom-dot {
    position: absolute; inset: 3px;
    border-radius: 50%;
    border: 1.5px solid rgba(255,255,255,0.7);
    pointer-events: none;
  }

  /* .btn.btn-primary covers colors/hover/disabled; a full-width button centres its label */
  .create-btn { width: 100%; justify-content: center; }
</style>

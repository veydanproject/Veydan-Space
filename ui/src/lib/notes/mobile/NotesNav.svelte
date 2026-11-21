<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { ask } from '$lib/core/ui/confirm.svelte';
  import { onMount, tick } from 'svelte';
  import { goto } from '$app/navigation';
  import Icon from '$lib/core/Icon.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import { api as shared } from '$lib/notes/api';
  import { api, formatError, type NoteListFilter, type NoteNav, type NavChild } from '$lib/notes/mobile/api';
  import { NAV_COLORS } from '$lib/core/mobile/nav-colors';
  import { longpress } from '$lib/core/mobile/longpress';
  import { t } from '$lib/core/mobile/i18n';
  import { compareLabel } from '$lib/core/label-order';
  import NavEditSheet from './NavEditSheet.svelte';
  import SmartViewSheet from './SmartViewSheet.svelte';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import { afterBacks } from '$lib/core/ui/sheetBack';

  interface Props {
    open: boolean;
    nav: NoteNav | null;
    filter: NoteListFilter;
    onclose: () => void;
    onselect: (filter: NoteListFilter) => void;
    onchange: () => void;
  }

  let { open, nav, filter, onclose, onselect, onchange }: Props = $props();
  let collapsed = $state<Set<string>>(new Set());
  let error = $state('');

  type Menu = { kind: 'folder' | 'smart'; id: string; name: string; color: string };
  let menu = $state<Menu | null>(null);

  let edit = $state<{ id: string | null; parentId?: string; title: string } | null>(null);
  let editName = $state('');
  let editColor = $state(NAV_COLORS[0]);
  let smartOpen = $state(false);
  let smartId = $state<string | null>(null);
  let saving = $state(false);
  let version = $state('');

  onMount(() => {
    void shared.system.hostInfo().then((h) => (version = h.version)).catch(() => {});
  });

  type FolderNode = { item: NavChild; children: FolderNode[]; total: number };

  // Folder tree. Children nest under their parent.
  const spaces = $derived.by((): FolderNode[] => {
    const folders = nav?.folders ?? [];
    const map = new Map<string, FolderNode>();
    for (const f of folders) map.set(f.id, { item: f, children: [], total: f.count });
    const roots: FolderNode[] = [];
    for (const node of map.values()) {
      const pid = node.item.parent_id;
      if (pid && map.has(pid)) map.get(pid)!.children.push(node);
      else roots.push(node);
    }
    roots.sort((a, b) => compareLabel(a.item.name, b.item.name));
    for (const node of map.values()) node.children.sort((a, b) => compareLabel(a.item.name, b.item.name));
    const rollup = (n: FolderNode): number => {
      n.total = n.item.count + n.children.reduce((s, c) => s + rollup(c), 0);
      return n.total;
    };
    roots.forEach(rollup);
    return roots;
  });

  // The section lists workspaces; Global is a top filter, as on the desktop. Without a
  // workspace (Notes alone, or none synced yet) there is no section at all.
  const hasBrowser = $derived(!!nav && nav.workspaces.length > 0);
  const workspaces = $derived([...(nav?.workspaces ?? [])].sort((a, b) => compareLabel(a.name, b.name)));

  function swatch(color: string | undefined): string {
    const c = (color ?? '').trim();
    return c && c.toLowerCase() !== '#ffffff' && c.toLowerCase() !== '#fff' ? c : 'var(--accent)';
  }

  function toggle(key: string) {
    const next = new Set(collapsed);
    if (next.has(key)) next.delete(key);
    else next.add(key);
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

  function folderChildKeys(): string[] {
    const keys: string[] = [];
    const walk = (nodes: FolderNode[]) => {
      for (const node of nodes) {
        if (node.children.length > 0) keys.push(`folder:${node.item.id}`);
        walk(node.children);
      }
    };
    walk(spaces);
    return keys;
  }

  // The drawer closes first and its place in the history is given back; only
  // then the page goes on: a goto under that back was undone by it (home, or
  // the screen before). A second tap while it closes does nothing.
  async function leave(then: () => void) {
    if (!open) return;
    onclose();
    await tick();
    await afterBacks();
    then();
  }

  function pick(kind: string, id?: string) {
    void leave(() => onselect({ kind, id }));
  }

  function active(kind: string, id?: string) {
    return filter.kind === kind && (id ?? '') === (filter.id ?? '');
  }

  function openNewFolder(parentId?: string) {
    editName = '';
    editColor = NAV_COLORS[0];
    edit = { id: null, parentId, title: parentId ? $t('notes_folder_sub') : $t('notes_folder_new') };
  }
  function openNewSmart() {
    smartId = null;
    smartOpen = true;
  }

  function startEditFromMenu() {
    if (!menu) return;
    if (menu.kind === 'smart') {
      smartId = menu.id;
      smartOpen = true;
      menu = null;
      return;
    }
    editName = menu.name;
    editColor = menu.color || NAV_COLORS[0];
    edit = { id: menu.id, title: $t('common_edit') };
    menu = null;
  }

  async function saveEdit(name: string, color: string) {
    if (!edit || saving) return;
    saving = true;
    error = '';
    try {
      if (edit.id) await api.notes.folderUpdate(edit.id, name, color);
      else {
        const f = await api.notes.folderCreate(name, edit.parentId, color);
        onselect({ kind: 'folder', id: f.id });
      }
      edit = null;
      onchange();
    } catch (e) {
      error = formatError(e);
    } finally {
      saving = false;
    }
  }

  async function removeCurrent() {
    if (!menu) return;
    if (!(await ask({ title: $t('notes_delete_item_confirm', { name: menu.name }) }))) return;
    const { kind, id } = menu;
    menu = null;
    error = '';
    try {
      if (kind === 'folder') await api.notes.folderDelete(id);
      else await api.notes.smartViewDelete(id);
      if (filter.kind === kind && filter.id === id) onselect({ kind: 'all' });
      onchange();
    } catch (e) {
      error = formatError(e);
    }
  }
</script>

<Drawer {open} side="left" width="min(88vw, 340px)" title={$t('app_notes')} {onclose} bodyPadding="0 var(--sp-3) calc(var(--sab) + var(--sp-3))">
  {#if error}
    <div class="err"><Notice tone="danger">{error}</Notice></div>
  {/if}

  {#if nav}
    <nav>
      <section>
        <button class="item" class:on={active('all')} onclick={() => pick('all')}>
          <Icon name="notes" size={18} />
          <span class="name">{$t('notes_filter_all')}</span>
          <span class="n">{nav.counts.all}</span>
        </button>
        {#if nav.counts.global > 0 && (hasBrowser || nav.counts.global !== nav.counts.all)}
          <button class="item" class:on={active('global')} onclick={() => pick('global')}>
            <Icon name="world" size={18} />
            <span class="name">{$t('notes_filter_global')}</span>
            <span class="n">{nav.counts.global}</span>
          </button>
        {/if}
        <button class="item" class:on={active('pinned')} onclick={() => pick('pinned')}>
          <Icon name="pin" size={18} />
          <span class="name">{$t('notes_filter_pinned')}</span>
          <span class="n">{nav.counts.pinned}</span>
        </button>
        <button class="item" class:on={active('archived')} onclick={() => pick('archived')}>
          <Icon name="archive" size={18} />
          <span class="name">{$t('notes_filter_archived')}</span>
          <span class="n">{nav.counts.archived}</span>
        </button>
        <button class="item" class:on={active('trash')} onclick={() => pick('trash')}>
          <Icon name="trash-2" size={18} />
          <span class="name">{$t('notes_filter_trash')}</span>
          <span class="n">{nav.counts.trash}</span>
        </button>
      </section>

      {#if hasBrowser}
        <section>
          <div class="label-row">
            <span class="section-label">{$t('notes_browser_section')}</span>
            <span class="head-actions">
              {@render foldChevs('browser', () => workspaces.filter((ws) => (ws.profiles ?? []).length > 0).map((ws) => `ws:${ws.id}`))}
              <span class="plus-slot"></span>
            </span>
          </div>
          {#if !collapsed.has('browser')}
            {#each workspaces as ws (ws.id)}
              {@const profiles = (ws.profiles ?? []).toSorted((a, b) => compareLabel(a.name, b.name))}
              <div class="ws-row">
                <button class="item" class:on={active('workspace', ws.id)} onclick={() => pick('workspace', ws.id)}>
                  <span class="m-dot" style:background={swatch(ws.color)}></span>
                  <span class="name">{ws.name}</span>
                  {#if ws.count > 0}<span class="n">{ws.count}</span>{/if}
                </button>
                {#if profiles.length > 0}
                  <button type="button" class="ws-chev" onclick={() => toggle(`ws:${ws.id}`)} aria-label={$t('notes_ws_fold_profiles')}>
                    <Icon name={collapsed.has(`ws:${ws.id}`) ? 'chevron-right' : 'chevron-down'} size={16} />
                  </button>
                {/if}
              </div>
              {#if !collapsed.has(`ws:${ws.id}`)}
                {#each profiles as p (p.id)}
                  <button class="item child" class:on={active('profile', p.id)} onclick={() => pick('profile', p.id)}>
                    <Icon name="browser" size={16} />
                    <span class="name">{p.name}</span>
                    <span class="n">{p.count}</span>
                  </button>
                {/each}
              {/if}
            {/each}
          {/if}
        </section>
      {/if}

      <section>
        <div class="label-row">
          <span class="section-label">{$t('notes_filter_folders')}</span>
          <span class="head-actions">
            {@render foldChevs('folders', folderChildKeys)}
            <span class="plus-slot">
              <button type="button" class="icon-btn" onclick={() => openNewFolder()} aria-label={$t('notes_create_folder')}>
                <Icon name="plus" size={22} />
              </button>
            </span>
          </span>
        </div>
        {#if !collapsed.has('folders')}
        {#each spaces as node (node.item.id)}
          {@render space(node)}
        {/each}
        {#if !spaces.length}
          <p class="empty-state sm">{$t('notes_filter_folders_empty')}</p>
        {/if}
        {/if}
      </section>

      <section>
        <div class="label-row">
          <span class="section-label">{$t('notes_smart_title')}</span>
          <span class="head-actions">
            {@render foldChevs('smart', () => [])}
            <span class="plus-slot">
              <button type="button" class="icon-btn" onclick={openNewSmart} aria-label={$t('notes_smart_new')}>
                <Icon name="plus" size={22} />
              </button>
            </span>
          </span>
        </div>
        {#if !collapsed.has('smart')}
        {#each nav.smart as view (view.id)}
          <button
            class="item"
            class:on={active('smart', view.id)}
            onclick={() => pick('smart', view.id)}
            {@attach longpress(() => (menu = { kind: 'smart', id: view.id, name: view.name, color: view.color }))}
          >
            <span class="m-doc" style:--c={view.color}><Icon name="list" size={14} /></span>
            <span class="name">{view.name}</span>
          </button>
        {/each}
        {#if !nav.smart.length}
          <p class="empty-state sm">{$t('notes_smart_empty')}</p>
        {/if}
        {/if}
      </section>

      {#if nav.sites.length}
        <section>
          <div class="label-row">
            <span class="section-label">{$t('notes_filter_sites')}</span>
            <span class="head-actions">
              {@render foldChevs('sites', () => [])}
              <span class="plus-slot"></span>
            </span>
          </div>
          {#if !collapsed.has('sites')}
          {#each nav.sites as s (s.id)}
            <button class="item" class:on={active('domain', s.id)} onclick={() => pick('domain', s.id)}>
              <Icon name="world" size={18} />
              <span class="name">{s.name}</span>
              <span class="n">{s.count}</span>
            </button>
          {/each}
          {/if}
        </section>
      {/if}

      <section>
        <button type="button" class="label-row" onclick={() => void leave(() => void goto('/notes/tags'))}>
          <span class="section-label">{$t('notes_tags')}</span>
          <Icon name="chevron-right" size={16} />
        </button>
      </section>

      <div class="brand">
        <img src="/logo.png" alt="" />
        <div class="brand-text">
          <span class="brand-name">{$t('app_name')}</span>
          {#if version}<span class="brand-ver">v{version}</span>{/if}
        </div>
      </div>
    </nav>
  {/if}
</Drawer>

<BottomSheet open={!!menu} title={menu?.name ?? ''} onclose={() => (menu = null)}>
  <div class="m-list">
    <button type="button" class="list-row dense" onclick={startEditFromMenu}><Icon name="pencil" size={20} /><span class="list-row-label">{$t('common_edit')}</span></button>
    {#if menu?.kind === 'folder' && !nav?.folders.find((f) => f.id === menu?.id)?.parent_id}
      <button type="button" class="list-row dense" onclick={() => { const id = menu!.id; menu = null; openNewFolder(id); }}><Icon name="folder-plus" size={20} /><span class="list-row-label">{$t('notes_folder_sub')}</span></button>
    {/if}
    <button type="button" class="list-row dense" onclick={removeCurrent}><Icon name="trash-2" size={20} /><span class="list-row-label danger">{$t('common_delete')}</span></button>
  </div>
</BottomSheet>

<NavEditSheet
  open={!!edit}
  title={edit?.title ?? ''}
  bind:name={editName}
  bind:color={editColor}
  placeholder={$t('notes_folder_name')}
  {saving}
  onclose={() => (edit = null)}
  onsave={saveEdit}
/>

{#if nav}
  <SmartViewSheet
    open={smartOpen}
    viewId={smartId}
    {nav}
    onclose={() => (smartOpen = false)}
    onsaved={(id) => { onselect({ kind: 'smart', id }); onchange(); }}
  />
{/if}

{#snippet foldChevs(sectionKey: string, childKeys: () => string[])}
  {@const nested = childKeys()}
  {@const folded = keysFolded(nested)}
  <span class="ws-chevs">
    {#if nested.length > 0}
      <button
        type="button"
        class="label-hit"
        onclick={() => toggleKeys(nested)}
        aria-label={sectionKey === 'browser'
          ? (folded ? $t('notes_ws_unfold_profiles') : $t('notes_ws_fold_profiles'))
          : (folded ? $t('notes_section_unfold_all') : $t('notes_section_fold_all'))}
      >
        <Icon name={folded ? 'chevrons-right' : 'chevrons-down'} size={16} />
      </button>
    {:else}
      <span class="label-hit"></span>
    {/if}
    <button
      type="button"
      class="label-hit"
      onclick={() => toggle(sectionKey)}
      aria-label={sectionKey === 'browser'
        ? (collapsed.has(sectionKey) ? $t('notes_ws_unfold') : $t('notes_ws_fold'))
        : (collapsed.has(sectionKey) ? $t('notes_section_unfold') : $t('notes_section_fold'))}
    >
      <Icon name={collapsed.has(sectionKey) ? 'chevron-right' : 'chevron-down'} size={16} />
    </button>
  </span>
{/snippet}

{#snippet space(node: FolderNode)}
  {@const f = node.item}
  <div class="space">
    <div class="ws-row">
      <button
        class="item space-row"
        class:on={active('folder', f.id)}
        onclick={() => pick('folder', f.id)}
        {@attach longpress(() => (menu = { kind: 'folder', id: f.id, name: f.name, color: f.color }))}
      >
        <span class="m-folder tinted-text" style:--chip={swatch(f.color)}><Icon name="folder" size={18} /></span>
        <span class="name">{f.name}</span>
        <span class="n">{node.total}</span>
      </button>
      {#if node.children.length > 0}
        <button type="button" class="ws-chev" onclick={() => toggle(`folder:${f.id}`)} aria-label={collapsed.has(`folder:${f.id}`) ? $t('notes_section_unfold') : $t('notes_section_fold')}>
          <Icon name={collapsed.has(`folder:${f.id}`) ? 'chevron-right' : 'chevron-down'} size={16} />
        </button>
      {/if}
    </div>
    {#if !collapsed.has(`folder:${f.id}`)}
      {#each node.children as child (child.item.id)}
        {@render folderNode(child, 1, f.color)}
      {/each}
    {/if}
  </div>
{/snippet}

{#snippet folderNode(node: FolderNode, depth: number, spaceColor: string)}
  {@const f = node.item}
  <button
    class="item folder-row"
    class:on={active('folder', f.id)}
    style:padding-left="{12 + depth * 20}px"
    onclick={() => pick('folder', f.id)}
    {@attach longpress(() => (menu = { kind: 'folder', id: f.id, name: f.name, color: f.color }))}
  >
    <span class="m-folder tinted-text" style:--chip={f.color || spaceColor}><Icon name="folder" size={18} /></span>
    <span class="name">{f.name}</span>
    <span class="n">{node.total}</span>
  </button>
  {#if !collapsed.has(`folder:${f.id}`)}
    {#each node.children as child (child.item.id)}
      {@render folderNode(child, depth + 1, spaceColor)}
    {/each}
  {/if}
{/snippet}

<style>
  .err { padding-top: var(--sp-3); }
  section { display: flex; flex-direction: column; }
  section + section { margin-top: var(--sp-3); }
  .item {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 56px; /* the phone's row of the drawer, as a dense row (base.css .list-row.dense) */
    padding: 0 12px;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-body);
    font-weight: var(--fw-medium);
    text-align: left;
    width: 100%;
    border-radius: var(--radius-md);
  }
  .item > :global(svg) { color: var(--text-2); }
  .item:active { background: var(--surface-2); }
  .item.on { background: var(--accent-bg); color: var(--accent-text); }
  .item.on > :global(svg) { color: var(--accent-text); }
  .item.child { padding-left: 40px; }
  .space-row { font-weight: var(--fw-bold); }
  .folder-row { font-weight: var(--fw-medium); }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .n {
    margin-left: auto;
    color: var(--text-3);
    font-size: var(--fs-sm);
    font-weight: var(--fw-semibold);
    flex-shrink: 0;
  }
  .item.on .n { color: inherit; }
  .space { display: flex; flex-direction: column; }
  .plus-slot {
    width: 40px;
    height: 40px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .head-actions { display: inline-flex; align-items: center; gap: 4px; margin-left: auto; }
  .head-actions .ws-chevs { margin-left: 0; }
  .ws-chevs { display: inline-flex; align-items: center; gap: 12px; margin-left: auto; }
  .label-hit {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 40px;
    min-height: 40px;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
  }
  .ws-row { display: flex; align-items: center; }
  .ws-row .item { flex: 1; min-width: 0; }
  .ws-chev {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border: 0;
    background: none;
    color: var(--text-3);
    flex-shrink: 0;
  }
  .label-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    /* The space around a group's .section-label is its row's: 40 high, the label in its middle, as its buttons. */
    min-height: 40px;
    border: 0;
    background: none;
    padding: 0 0 0 var(--sp-1);
    text-align: left;
    cursor: pointer;
  }
  .label-row .section-label { flex: 1; }
  .label-row > :global(svg) { color: var(--text-3); flex-shrink: 0; margin-right: var(--sp-2-5); }
  .brand {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2-5);
    margin-top: var(--sp-4);
    padding: var(--sp-3) var(--sp-2) 0;
    border-top: 1px solid var(--border);
  }
  .brand img {
    width: 28px;
    height: 28px;
    object-fit: contain;
    flex-shrink: 0;
  }
  .brand-text { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .brand-name {
    font-size: var(--fs-sm);
    font-weight: var(--fw-bold);
    color: var(--text-2);
  }
  .brand-ver {
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
    color: var(--text-3);
  }
  .danger { color: var(--danger-text); }
</style>

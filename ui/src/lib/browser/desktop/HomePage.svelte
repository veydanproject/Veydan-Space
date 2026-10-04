<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { api } from '$lib/browser/api';
  import { t, locale, countKey } from '$lib/core/i18n';
  import type { Workspace, WorkspaceStats, CreateWorkspaceRequest } from '$lib/browser/types';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { workspacesStore } from '$lib/browser/store/workspaces.svelte';
  import { profilesStore } from '$lib/browser/store/profiles.svelte';
  import { proxiesStore } from '$lib/browser/store/proxies.svelte';
  import { runningStore } from '$lib/browser/store/running.svelte';
  import { formatError } from '$lib/core/utils';

  const ORDER_KEY = 'ws_order';

  // The cards follow the store (demo data, a sync from another device, the
  // dialogs); only the order the user dragged them into is the page's own.
  let order = $state<string[]>(readOrder());
  const workspaces = $derived(applyOrder(workspacesStore.list, order));
  let stats = $state<Record<string, WorkspaceStats>>({});
  let loading = $state(false);
  let error = $state('');

  let createModal = $state(false);
  let editModal = $state<{ open: boolean; id: string; name: string; description: string; color: string }>({
    open: false, id: '', name: '', description: '', color: '#8b7bff',
  });
  let deleteModal = $state<{ open: boolean; id: string; name: string }>({
    open: false, id: '', name: '',
  });
  let createForm = $state<CreateWorkspaceRequest>({ name: '', description: '', color: '#8b7bff' });
  let saving = $state(false);

  let dragSrcIndex = $state(-1);
  let insertAt = $state(-1);
  let ghostActive = $state(false);
  let ghostX = $state(0);
  let ghostY = $state(0);

  // Category palette from the redesign (workspace accents); stored per-workspace as data.
  const COLORS = [
    '#8b7bff', '#60a5fa', '#2dd4bf', '#f472b6',
    '#f5c451', '#34d399', '#f26d6d', '#f97316',
  ];

  onMount(() => {
    // The cached list is shown at once; the store refreshes it in the background.
    if (!workspacesStore.loaded) void loadData();
  });

  // The counters of the cards are the backend's: they are asked again whenever
  // the workspaces, the profiles, the proxies or the running profiles change.
  $effect(() => {
    const ids = workspaces.map((w) => w.id);
    void profilesStore.list;
    void proxiesStore.list;
    void runningStore.ids;
    untrack(() => void loadStats(ids));
  });

  function readOrder(): string[] {
    try {
      const saved: unknown = JSON.parse(localStorage.getItem(ORDER_KEY) ?? '[]');
      return Array.isArray(saved) ? saved.filter((id): id is string => typeof id === 'string') : [];
    } catch {
      return [];
    }
  }

  function applyOrder(ws: Workspace[], saved: string[]): Workspace[] {
    if (saved.length === 0) return ws;
    const map = new Map(ws.map((w) => [w.id, w]));
    const ordered = saved.map((id) => map.get(id)).filter(Boolean) as Workspace[];
    const rest = ws.filter((w) => !saved.includes(w.id));
    return [...ordered, ...rest];
  }

  function saveOrder(ids: string[]) {
    order = ids;
    try {
      localStorage.setItem(ORDER_KEY, JSON.stringify(ids));
    } catch {}
  }

  function findInsertIndex(clientX: number, clientY: number): number {
    const cards = [...document.querySelectorAll('.workspace-card')] as HTMLElement[];
    if (cards.length === 0) return 0;
    let minDist = Infinity;
    let result = cards.length;
    for (let idx = 0; idx < cards.length; idx++) {
      const rect = cards[idx].getBoundingClientRect();
      const cx = rect.left + rect.width / 2;
      const cy = rect.top + rect.height / 2;
      const dist = Math.hypot(clientX - cx, clientY - cy);
      if (dist < minDist) {
        minDist = dist;
        result = clientX < cx ? idx : idx + 1;
      }
    }
    return result;
  }

  function onHandlePointerDown(e: PointerEvent, i: number) {
    e.preventDefault();
    dragSrcIndex = i;
    const startX = e.clientX;
    const startY = e.clientY;
    let moved = false;

    function onMove(ev: PointerEvent) {
      if (!moved && Math.hypot(ev.clientX - startX, ev.clientY - startY) > 6) {
        moved = true;
        ghostActive = true;
        document.body.style.cursor = 'grabbing';
      }
      if (moved) {
        ghostX = ev.clientX;
        ghostY = ev.clientY;
        insertAt = findInsertIndex(ev.clientX, ev.clientY);
      }
    }

    function onUp() {
      document.removeEventListener('pointermove', onMove);
      document.removeEventListener('pointerup', onUp);
      document.body.style.cursor = '';

      if (moved && dragSrcIndex !== -1) {
        const from = dragSrcIndex;
        const to = insertAt;
        if (to !== -1 && from !== to && from !== to - 1) {
          const arr = workspaces.map((w) => w.id);
          const [item] = arr.splice(from, 1);
          arr.splice(to > from ? to - 1 : to, 0, item);
          saveOrder(arr);
        }
      }

      dragSrcIndex = -1;
      insertAt = -1;
      ghostActive = false;
    }

    document.addEventListener('pointermove', onMove);
    document.addEventListener('pointerup', onUp);
  }

  async function loadData() {
    loading = true;
    try {
      await workspacesStore.ensureLoaded();
    } catch (e) {
      error = formatError(e);
    } finally {
      loading = false;
    }
  }

  /** Bumped per request so an older answer cannot overwrite a newer one. */
  let statsGeneration = 0;

  async function loadStats(ids: string[]) {
    const generation = ++statsGeneration;
    try {
      const statsArr = await Promise.all(ids.map((id) => api.workspaces.stats(id)));
      if (generation !== statsGeneration) return;
      const map: Record<string, WorkspaceStats> = {};
      statsArr.forEach((s) => (map[s.id] = s));
      stats = map;
    } catch (e) {
      if (generation === statsGeneration) error = formatError(e);
    }
  }

  async function createWorkspace() {
    if (!createForm.name.trim()) return;
    saving = true;
    try {
      const w = await workspacesStore.create(createForm);
      stats = { ...stats, [w.id]: { id: w.id, profile_count: 0, proxy_count: 0, active_count: 0 } };
      saveOrder(workspaces.map((ws) => ws.id));
      createModal = false;
      createForm = { name: '', description: '', color: '#8b7bff' };
    } catch (e) {
      error = formatError(e);
    } finally {
      saving = false;
    }
  }

  function openEdit(w: Workspace) {
    editModal = { open: true, id: w.id, name: w.name, description: w.description ?? '', color: w.color };
  }

  async function saveEdit() {
    saving = true;
    try {
      await workspacesStore.update(editModal.id, {
        name: editModal.name,
        description: editModal.description || null,
        color: editModal.color,
      });
      editModal = { open: false, id: '', name: '', description: '', color: '#8b7bff' };
    } catch (e) {
      error = formatError(e);
    } finally {
      saving = false;
    }
  }

  function openDelete(w: Workspace) {
    deleteModal = { open: true, id: w.id, name: w.name };
  }

  async function confirmDelete(mode: 'move_to_default' | 'delete_all') {
    try {
      await workspacesStore.remove(deleteModal.id, mode);
      deleteModal = { open: false, id: '', name: '' };
      await profilesStore.refresh();
      await proxiesStore.refresh();
    } catch (e) {
      error = formatError(e);
    }
  }

  function getStats(id: string) {
    return stats[id] ?? { profile_count: 0, proxy_count: 0, active_count: 0 };
  }
</script>

<div class="page">
  <div class="page-header">
    <div class="page-title-group">
      <h1>{$t('workspaces_title')}</h1>
      <p class="page-sub">{$t(countKey('workspaces_sub', workspaces.length, $locale), { count: String(workspaces.length) })}</p>
    </div>
    <button class="btn btn-primary spacer" onclick={() => (createModal = true)}>
      <Icon name="plus" size={15} />{$t('workspaces_new')}
    </button>
  </div>

  {#if error}
    <div class="error-msg" style="margin-bottom:1rem">{error}</div>
  {/if}

  {#if loading}
    <div class="empty-state">{$t('loading')}</div>
  {:else if workspaces.length === 0}
    <div class="empty-state">
      <div class="empty-icon"><Icon name="layers" size={48} strokeWidth={1.25} /></div>
      <p>{$t('workspaces_empty')}</p>
      <button class="btn btn-primary" onclick={() => (createModal = true)}>
        {$t('workspaces_empty_create')}
      </button>
    </div>
  {:else}
    <div class="workspace-grid">
      {#each workspaces as ws, i (ws.id)}
        {@const s = getStats(ws.id)}
        <div
          class="workspace-card"
          class:drag-source={dragSrcIndex === i}
          class:insert-before={ghostActive && insertAt === i && dragSrcIndex !== i && dragSrcIndex !== i - 1}
          style="--ws-color: {ws.color}"
          role="button"
          tabindex="0"
          onclick={() => !ghostActive && goto(`/workspace/${ws.id}`)}
          onkeydown={(e) => e.key === 'Enter' && e.target === e.currentTarget && goto(`/workspace/${ws.id}`)}
        >
          {#if !ws.is_default}
            <div class="card-menu">
              <button class="menu-btn" onclick={(e) => { e.stopPropagation(); openEdit(ws); }} title={$t('workspaces_btn_edit')}>
                <Icon name="pencil" size={13} />
              </button>
              <button class="menu-btn danger" onclick={(e) => { e.stopPropagation(); openDelete(ws); }} title={$t('workspaces_btn_delete')}>
                <Icon name="trash-2" size={13} />
              </button>
            </div>
          {/if}
          <div class="card-accent"></div>
          <div class="card-body">
            <div class="card-header">
              <div class="drag-handle" role="button" tabindex="-1" aria-label={$t('workspaces_drag')} title={$t('workspaces_drag')} onpointerdown={(e) => onHandlePointerDown(e, i)}>
                <Icon name="grip-vertical" size={14} />
              </div>
              <div class="card-icon" style="background: color-mix(in srgb, {ws.color} 14%, transparent)">
                <Icon name="layers" size={18} />
              </div>
              <div class="card-title-group">
                <h2 class="card-title">{ws.name}</h2>
                {#if ws.description}
                  <p class="card-desc">{ws.description}</p>
                {/if}
              </div>
            </div>

            <div class="card-stats">
              <div class="stat">
                <Icon name="folder-open" size={13} />
                <span>{$t(countKey('workspaces_profiles_n', s.profile_count, $locale), { n: String(s.profile_count) })}</span>
              </div>
              <div class="stat">
                <Icon name="globe" size={13} />
                <span>{$t(countKey('workspaces_proxies_n', s.proxy_count, $locale), { n: String(s.proxy_count) })}</span>
              </div>
              {#if s.active_count > 0}
                <div class="stat active">
                  <span class="active-dot"></span>
                  <span>{$t(countKey('workspaces_active_n', s.active_count, $locale), { n: String(s.active_count) })}</span>
                </div>
              {/if}
            </div>
          </div>

          <div class="card-footer">
            <a href="/workspace/{ws.id}" class="open-btn" onclick={(e) => { e.preventDefault(); e.stopPropagation(); void goto(`/workspace/${ws.id}`); }}>
              {$t('workspaces_open')}
              <Icon name="arrow-right" size={16} />
            </a>
          </div>
        </div>
      {/each}

      <button class="add-card" onclick={() => (createModal = true)}>
        <Icon name="plus" size={24} strokeWidth={1.5} />
        <span>{$t('workspaces_new')}</span>
      </button>
    </div>
  {/if}
</div>

{#if ghostActive && dragSrcIndex !== -1}
  <div
    class="drag-ghost"
    style="left:{ghostX}px; top:{ghostY}px; --ws-color:{workspaces[dragSrcIndex]?.color ?? '#8b7bff'}"
  >
    <div class="card-accent"></div>
    <div class="ghost-name">{workspaces[dragSrcIndex]?.name}</div>
  </div>
{/if}

<!-- Create Workspace Modal -->
<Dialog bind:open={createModal} title={$t('workspaces_new')}>
  <div class="ws-form">
  <div class="form-group">
    <label for="ws-name">{$t('workspaces_form_name')}</label>
    <!-- svelte-ignore a11y_autofocus -->
    <input
      id="ws-name"
      type="text"
      bind:value={createForm.name}
      placeholder={$t('workspaces_form_name_placeholder')}
      autofocus
    />
  </div>
  <div class="form-group">
    <label for="ws-desc">{$t('workspaces_form_desc')}</label>
    <input id="ws-desc" type="text" bind:value={createForm.description} placeholder="…" />
  </div>
  <div class="form-group">
    <label for="ws-color">{$t('workspaces_form_color')}</label>
    <div id="ws-color" class="color-picker" role="group" aria-label={$t('workspaces_form_color')}>
      {#each COLORS as c}
        <button
          class="color-swatch"
          class:selected={createForm.color === c}
          style="background: {c}"
          aria-label={c}
          aria-pressed={createForm.color === c}
          onclick={() => (createForm.color = c)}
        ></button>
      {/each}
      <label
        class="color-swatch color-swatch-custom"
        class:selected={!COLORS.includes(createForm.color ?? '')}
        title={$t('workspaces_color_custom')}
        aria-label={$t('workspaces_color_custom')}
      >
        <input type="color" bind:value={createForm.color} />
        {#if !COLORS.includes(createForm.color ?? '')}
          <span class="custom-dot" style="background:{createForm.color}"></span>
        {/if}
      </label>
    </div>
  </div>
  </div>
  {#snippet footer()}
    <button class="btn btn-ghost" onclick={() => (createModal = false)}>{$t('workspaces_btn_cancel')}</button>
    <button class="btn btn-primary" disabled={saving || !createForm.name.trim()} onclick={createWorkspace}>
      {saving ? '…' : $t('workspaces_btn_create')}
    </button>
  {/snippet}
</Dialog>

<!-- Edit Workspace Modal -->
<Dialog open={editModal.open} title={$t('workspaces_btn_edit')} onclose={() => (editModal = { ...editModal, open: false })}>
  <div class="ws-form">
  <div class="form-group">
    <label for="edit-name">{$t('workspaces_form_name')}</label>
    <input id="edit-name" type="text" bind:value={editModal.name} />
  </div>
  <div class="form-group">
    <label for="edit-desc">{$t('workspaces_form_desc')}</label>
    <input id="edit-desc" type="text" bind:value={editModal.description} />
  </div>
  <div class="form-group">
    <label for="edit-color">{$t('workspaces_form_color')}</label>
    <div id="edit-color" class="color-picker" role="group" aria-label={$t('workspaces_form_color')}>
      {#each COLORS as c}
        <button
          class="color-swatch"
          class:selected={editModal.color === c}
          style="background: {c}"
          aria-label={c}
          aria-pressed={editModal.color === c}
          onclick={() => (editModal.color = c)}
        ></button>
      {/each}
      <label
        class="color-swatch color-swatch-custom"
        class:selected={!COLORS.includes(editModal.color)}
        title={$t('workspaces_color_custom')}
        aria-label={$t('workspaces_color_custom')}
      >
        <input type="color" bind:value={editModal.color} />
        {#if !COLORS.includes(editModal.color)}
          <span class="custom-dot" style="background:{editModal.color}"></span>
        {/if}
      </label>
    </div>
  </div>
  </div>
  {#snippet footer()}
    <button class="btn btn-ghost" onclick={() => (editModal = { ...editModal, open: false })}>{$t('workspaces_btn_cancel')}</button>
    <button class="btn btn-primary" disabled={saving} onclick={saveEdit}>
      {saving ? '…' : $t('workspaces_btn_save')}
    </button>
  {/snippet}
</Dialog>

<!-- Delete Workspace Modal -->
<Dialog open={deleteModal.open} title={$t('workspaces_btn_delete')} onclose={() => (deleteModal = { ...deleteModal, open: false })}>
  <p class="delete-warning">
    <Icon name="alert-triangle" size={16} />
    <!-- one flex item: the gap of the row stays between the icon and the text -->
    <span>{#each $t('workspaces_delete_question').split('{name}') as part, i}{#if i > 0}<strong>{deleteModal.name}</strong>{/if}{part}{/each}</span>
  </p>
  <div class="delete-options">
    <button class="delete-option" onclick={() => confirmDelete('move_to_default')}>
      <Icon name="arrow-left" size={14} />
      <div>
        <div class="option-title">{$t('workspaces_delete_move')}</div>
        <div class="option-desc">{$t('workspaces_delete_move_hint')}</div>
      </div>
    </button>
    <button class="delete-option danger" onclick={() => confirmDelete('delete_all')}>
      <Icon name="trash-2" size={14} />
      <div>
        <div class="option-title">{$t('workspaces_delete_all')}</div>
        <div class="option-desc">{$t('workspaces_delete_all_hint')}</div>
      </div>
    </button>
  </div>
  {#snippet footer()}
    <button class="btn btn-ghost" onclick={() => (deleteModal = { ...deleteModal, open: false })}>{$t('workspaces_btn_cancel')}</button>
  {/snippet}
</Dialog>

<style>
  /* Screen entry animation (design: vfade) */
  .page { animation: vfade 0.25s ease; }
  .page-title-group { display: flex; flex-direction: column; gap: 6px; }

  /* ── Grid ── */
  .workspace-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 18px;
  }

  /* ── Card ── */
  .workspace-card {
    position: relative;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    overflow: hidden;
    display: flex;
    flex-direction: column;
    transition: border-color 0.18s, transform 0.18s, box-shadow 0.18s;
    cursor: pointer;
  }

  .workspace-card:hover {
    border-color: var(--border-2);
    transform: translateY(-2px);
  }
  .workspace-card.drag-source { opacity: 0.25; transition: none; }
  .workspace-card.insert-before { box-shadow: -4px 0 0 0 var(--accent), var(--shadow-lg); }

  .drag-ghost {
    position: fixed;
    width: 220px;
    background: var(--bg-2);
    border: 1.5px solid var(--ws-color, var(--accent));
    border-radius: var(--radius);
    box-shadow: var(--shadow-lg);
    pointer-events: none;
    z-index: 9999;
    opacity: 0.93;
    transform: translate(-50%, -50%) rotate(2deg) scale(1.04);
    overflow: hidden;
    user-select: none;
  }
  .ghost-name {
    padding: 0.625rem 0.875rem;
    font-size: var(--fs-sm);
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* The hover controls take no room from the title: the handle sits in the
     card's left padding beside the icon, the menu in the free top-right
     corner level with the colour tick. */
  .drag-handle {
    position: absolute; left: -20px; top: 0;
    display: flex; align-items: center; justify-content: center;
    color: var(--text-3); cursor: grab;
    width: 18px; height: 44px; opacity: 0; transition: opacity 0.15s;
  }
  .workspace-card:hover .drag-handle { opacity: 1; }
  .drag-handle:active { cursor: grabbing; }

  /* Colour tick (design: 44×3 rounded bar at the card top, not a full strip) */
  .card-accent {
    width: 44px;
    height: 3px;
    border-radius: 3px;
    margin: 22px 0 0 22px;
    background: var(--ws-color, var(--accent));
    flex-shrink: 0;
  }

  .card-body {
    padding: 18px 22px 4px;
    display: flex;
    flex-direction: column;
    gap: 1.1rem;
    flex: 1;
  }

  .card-header {
    position: relative;
    display: flex;
    align-items: flex-start;
    gap: var(--sp-3);
  }

  .card-icon {
    width: 44px;
    height: 44px;
    border-radius: var(--radius-md);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--ws-color, var(--accent));
    flex-shrink: 0;
  }

  .card-title-group { flex: 1; min-width: 0; }
  .card-title { font-size: var(--fs-md); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .card-desc { font-size: 0.78rem; color: var(--text-faint); margin-top: 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .card-menu {
    position: absolute; top: 10px; right: 12px;
    display: flex; gap: 2px; opacity: 0; transition: opacity 0.15s;
  }
  .workspace-card:hover .card-menu,
  .workspace-card:focus-within .card-menu { opacity: 1; }
  .menu-btn {
    display: flex; align-items: center; justify-content: center;
    width: 30px; height: 30px; background: transparent; border-radius: var(--radius-sm);
    border: none; color: var(--text-3); cursor: pointer; transition: all 0.15s;
  }
  .menu-btn:hover { background: var(--surface-2); color: var(--text-body); }
  .menu-btn.danger:hover { background: var(--danger-bg); color: var(--danger-text); }

  .card-stats { display: flex; align-items: center; gap: 22px; flex-wrap: wrap; }
  .stat { display: flex; align-items: center; gap: 7px; font-size: 0.82rem; color: var(--text-soft); }
  .stat :global(svg) { color: var(--text-dim); }
  .stat.active { color: var(--success-text); font-weight: 500; }
  .stat.active :global(svg) { color: var(--success); }
  .active-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--success); flex-shrink: 0; }

  .card-footer {
    border-top: 1px solid var(--border);
    margin: 14px 22px 0;
    padding: 14px 0 18px;
  }

  .open-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-base);
    font-weight: var(--fw-semibold);
    color: var(--accent-text-3);
    text-decoration: none;
    transition: gap 0.15s, color 0.15s;
  }
  .open-btn:hover { color: var(--accent-text); gap: 10px; }

  /* ── Add card ── */
  .add-card {
    background: transparent;
    border: 1.5px dashed var(--border-2);
    border-radius: var(--radius-lg);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    color: var(--text-3);
    cursor: pointer;
    min-height: 210px;
    transition: all 0.15s;
    font-size: var(--fs-base);
  }
  .add-card:hover { border-color: var(--border-2); color: var(--text-2); background: var(--surface); }

  /* ── Modal form layout (dialog body) ── */
  .ws-form { display: flex; flex-direction: column; gap: 0.875rem; }

  .color-picker { display: flex; gap: 0.4rem; flex-wrap: wrap; align-items: center; }
  .color-swatch {
    width: 24px; height: 24px; border-radius: 50%; border: 2px solid transparent;
    cursor: pointer; transition: transform 0.1s, border-color 0.1s;
    padding: 0; flex-shrink: 0;
  }
  .color-swatch:hover { transform: scale(1.15); }
  .color-swatch.selected { border-color: var(--text); transform: scale(1.15); }

  .color-swatch-custom {
    position: relative;
    background: conic-gradient(
      #f43f5e, #f97316, #eab308, #22c55e, #06b6d4, #6366f1, #ec4899, #f43f5e
    );
    display: flex; align-items: center; justify-content: center;
    overflow: hidden;
  }
  .color-swatch-custom input[type="color"] {
    position: absolute; inset: 0; width: 100%; height: 100%;
    opacity: 0; cursor: pointer; border: none; padding: 0; margin: 0;
  }
  .color-swatch-custom .custom-dot {
    position: absolute; inset: 3px; border-radius: 50%;
    border: 1.5px solid rgba(255,255,255,0.6);
    pointer-events: none;
  }

  .delete-warning {
    display: flex; align-items: center; gap: var(--sp-2);
    color: var(--warn-text); font-size: var(--fs-sm);
    background: var(--warn-bg); padding: 0.625rem 0.875rem;
    border-radius: var(--radius-sm);
    margin-bottom: var(--sp-3);
  }

  .delete-options { display: flex; flex-direction: column; gap: var(--sp-2); }
  .delete-option {
    display: flex; align-items: flex-start; gap: var(--sp-3);
    padding: var(--sp-3); border-radius: var(--radius-sm);
    background: var(--surface-2); border: 1px solid var(--border);
    cursor: pointer; text-align: left; transition: all 0.15s; color: var(--text);
  }
  .delete-option:hover { border-color: var(--border-2); background: var(--surface); }
  .delete-option.danger { color: var(--danger-text); }
  .delete-option.danger:hover { background: var(--danger-bg); border-color: color-mix(in srgb, var(--danger) 30%, var(--border)); }
  .option-title { font-size: var(--fs-sm); font-weight: 600; margin-bottom: 0.15rem; }
  .option-desc { font-size: var(--fs-xs); color: var(--text-2); }
</style>

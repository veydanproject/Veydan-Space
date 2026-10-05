<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { api } from '$lib/browser/api';
  import { t, locale, countKey, type TranslationKey } from '$lib/core/i18n';
  import type {
    Workspace,
    Profile,
    Proxy,
    WorkspaceColumn,
    ProxyCheckResult,
  } from '$lib/browser/types';
  import Icon from '$lib/core/Icon.svelte';
  import Modal from '$lib/core/ui/Modal.svelte';
  import KanbanBoard from '$lib/browser/components/KanbanBoard.svelte';
  import TableView from '$lib/browser/components/TableView.svelte';
  import TopologyGraph from '$lib/browser/components/TopologyGraph.svelte';
  import CreateProfilePanel from '$lib/browser/components/CreateProfilePanel.svelte';
  import EditProfilePanel from '$lib/browser/components/EditProfilePanel.svelte';
  import ProfileSidePanel from '$lib/browser/components/ProfileSidePanel.svelte';
  import RawDataPanel from '$lib/browser/components/RawDataPanel.svelte';
  import ImportProfileModal from '$lib/browser/components/ImportProfileModal.svelte';
  import ProxyPanel from '$lib/browser/components/ProxyPanel.svelte';
  import { listen } from '@tauri-apps/api/event';
  import { profilesStore } from '$lib/browser/store/profiles.svelte';
  import { proxiesStore } from '$lib/browser/store/proxies.svelte';
  import { formatError } from '$lib/core/utils';
  import { explainError } from '$lib/browser/tor-error';
  import ProxyAddress from '$lib/browser/components/ProxyAddress.svelte';
  import { registry } from '$lib/core/registry';

  let workspaceId = $derived($page.params.id ?? '');

  type Tab = 'board' | 'proxies' | 'topology' | 'notes';
  type ViewMode = 'kanban' | 'table';

  let tab = $state<Tab>('board');
  let viewMode = $state<ViewMode>('kanban');

  let workspace = $state<Workspace | null>(null);
  let profiles = $state<Profile[]>([]);
  let proxies = $state<Proxy[]>([]);
  let columns = $state<WorkspaceColumn[]>([]);
  let loading = $state(false);
  let error = $state('');
  let saving = $state(false);
  let notesValue = $state('');
  let notesSaving = $state(false);

  // Running profiles set (updated by events + fallback)
  let runningProfiles = $state<Set<string>>(new Set());

  let showCreatePanel = $state(false);
  let showImportModal = $state(false);
  let editingProfile = $state<import('$lib/browser/types').Profile | null>(null);
  let selectedProfile = $state<import('$lib/browser/types').Profile | null>(null);
  let rawDataProfile = $state<import('$lib/browser/types').Profile | null>(null);
  let unlistenRunning: (() => void) | undefined;
  let unlistenSync: (() => void) | undefined;

  const isSelectedRunning = $derived(
    selectedProfile ? runningProfiles.has(selectedProfile.id) : false
  );

  // Reload data whenever workspaceId changes (covers initial load + navigation between workspaces)
  $effect(() => {
    const id = workspaceId;
    if (!id) return;
    tab = 'board';
    workspace = null;
    profiles = [];
    proxies = [];
    columns = [];
    notesValue = '';
    viewMode = (localStorage.getItem(`rb-view-${id}`) as ViewMode) ?? 'kanban';
    loadData();
    refreshStatuses();
  });

  onMount(async () => {
    // Event listener lives for the full component lifecycle
    unlistenRunning = await listen<{ running_ids: string[] }>(
      'profiles://running-changed',
      (e) => { runningProfiles = new Set(e.payload.running_ids); }
    );
    unlistenSync = await listen<string[]>('sync://data-changed', (e) => {
      if (e.payload.some((x) => x === 'workspace_column' || x === 'profile' || x === 'workspace' || x === 'proxy')) {
        reloadWorkspace();
      }
    });
  });

  onDestroy(() => {
    unlistenRunning?.();
    unlistenSync?.();
  });

  async function reloadWorkspace() {
    if (!workspaceId) return;
    try {
      const [ws, cols] = await Promise.all([
        api.workspaces.get(workspaceId),
        api.workspaces.columns.list(workspaceId),
        profilesStore.refresh(),
        proxiesStore.refresh(),
      ]);
      if (!ws) return;
      workspace = ws;
      columns = cols;
      notesValue = ws.notes ?? '';
      profiles = profilesStore.byWorkspace(workspaceId);
      const selectedId = selectedProfile?.id;
      const rawId = rawDataProfile?.id;
      if (selectedId) {
        selectedProfile = profiles.find((p) => p.id === selectedId) ?? selectedProfile;
      }
      if (rawId) {
        rawDataProfile = profiles.find((p) => p.id === rawId) ?? rawDataProfile;
      }
      proxies = proxiesStore.byWorkspace(workspaceId);
    } catch {}
  }

  async function loadData() {
    loading = true;
    try {
      const [ws, cols] = await Promise.all([
        api.workspaces.get(workspaceId),
        api.workspaces.columns.list(workspaceId),
        profilesStore.ensureLoaded(),
        proxiesStore.ensureLoaded(),
      ]);
      if (!ws) { goto('/', { replaceState: true }); return; }
      workspace = ws;
      columns = cols;
      notesValue = ws.notes ?? '';
      profiles = profilesStore.byWorkspace(workspaceId);
      proxies = proxiesStore.byWorkspace(workspaceId);
    } catch (e) { error = formatError(e); }
    finally { loading = false; }
  }

  async function refreshStatuses() {
    try {
      const ids = await api.profiles.runningIds();
      runningProfiles = new Set(ids);
    } catch {}
  }

  function setViewMode(mode: ViewMode) {
    viewMode = mode;
    localStorage.setItem(`rb-view-${workspaceId}`, mode);
  }

  async function saveNotes() {
    if (!workspace) return;
    notesSaving = true;
    try {
      const updated = await api.workspaces.update(workspaceId, { notes: notesValue });
      workspace = updated;
    } catch (e) { error = formatError(e); }
    finally { notesSaving = false; }
  }

  async function stopAll() {
    saving = true;
    try {
      const ids = [...runningProfiles];
      await Promise.all(ids.map((id) => api.profiles.stop(id).catch(() => {})));
      await refreshStatuses();
    } catch (e) { error = formatError(e); }
    finally { saving = false; }
  }
  async function onProfileChange() {
    await profilesStore.refresh();
    profiles = profilesStore.byWorkspace(workspaceId);
  }
  function onColumnsChange() { /* columns already updated in child via $bindable */ }

  let runningCount = $derived(runningProfiles.size);

  // --- Proxy CRUD ---
  // The other modules' drawers for a workspace — TOTP, passwords, notes, SSH — behind the toolbar's buttons.
  const drawers = registry.views('workspace', 'drawer');
  let drawerOpen = $state<Record<string, boolean>>(Object.fromEntries(drawers.map((d) => [d.id, false])));
  let proxyPanelProxy = $state<Proxy | null | undefined>(undefined);
  let proxyCheckResults = $state<Record<string, ProxyCheckResult & { checking?: boolean; err?: string }>>({});
  let proxyDeleteModal = $state({ open: false, id: '', name: '', usage: '' });
  let proxySearch = $state('');

  function openCreateProxy() {
    proxyPanelProxy = null;
  }

  let fingerprintPrompt = $state<{ id: string; fingerprint: string; ip: string; country: string | null; city: string | null } | null>(null);

  async function checkProxy(id: string) {
    proxyCheckResults = { ...proxyCheckResults, [id]: { ip: '', country: null, city: null, ok: false, checking: true } };
    try {
      const result = await proxiesStore.check(id);
      proxyCheckResults = { ...proxyCheckResults, [id]: { ...result, checking: false } };

      if (result.ssh_fingerprint_is_new && result.ssh_fingerprint) {
        fingerprintPrompt = { id, fingerprint: result.ssh_fingerprint, ip: result.ip, country: result.country, city: result.city };
        return;
      }

      proxies = proxiesStore.byWorkspace(workspaceId);
    } catch (e) {
      proxyCheckResults = { ...proxyCheckResults, [id]: { ip: '', country: null, city: null, ok: false, checking: false, err: explainError(e, $t) } };
      proxiesStore.markFailed(id);
      proxies = proxiesStore.byWorkspace(workspaceId);
    }
  }

  async function trustFingerprint() {
    if (!fingerprintPrompt) return;
    const { id, fingerprint, ip, country, city } = fingerprintPrompt;
    try {
      await proxiesStore.trustFingerprint(id, fingerprint, ip, country, city);
      proxies = proxiesStore.byWorkspace(workspaceId);
    } catch (e) { console.error('trustFingerprint error:', e); }
    finally { fingerprintPrompt = null; }
  }

  /** Open the confirmation, then fill in what the delete would detach. */
  async function askDeleteProxy(proxy: Proxy) {
    proxyDeleteModal = { open: true, id: proxy.id, name: proxy.name, usage: '' };
    const usage = await proxiesStore.deleteWarning(proxy.id);
    // The user may have cancelled or picked another row while this was in flight.
    if (proxyDeleteModal.open && proxyDeleteModal.id === proxy.id) {
      proxyDeleteModal.usage = usage;
    }
  }

  async function confirmDeleteProxy() {
    try {
      await proxiesStore.remove(proxyDeleteModal.id);
      proxies = proxiesStore.byWorkspace(workspaceId);
    } catch {}
    finally { proxyDeleteModal = { open: false, id: '', name: '', usage: '' }; }
  }

  function onProxyPanelSaved(proxy: Proxy) {
    proxiesStore.upsert(proxy);
    proxies = proxiesStore.byWorkspace(workspaceId);
    proxyPanelProxy = undefined;
  }

  let filteredProxies = $derived(proxies.filter((p) => {
    if (!proxySearch.trim()) return true;
    const q = proxySearch.toLowerCase();
    return (
      p.name.toLowerCase().includes(q) ||
      p.host.toLowerCase().includes(q) ||
      (p.country ?? '').toLowerCase().includes(q) ||
      (p.last_ip ?? '').includes(q)
    );
  }));
</script>

<div class="page page--fill ws-page">
  {#if loading}
    <div class="centered">{$t('loading')}</div>
  {:else if workspace}
    <!-- Header -->
    <div class="ws-header">
      <div class="breadcrumb">
        <a href="/" class="back-link">
          <Icon name="arrow-left" size={15} />
          {$t('back_workspaces')}
        </a>
        <span class="crumb-sep">/</span>
        <span class="ws-name">{workspace.name}</span>
      </div>

      <div class="ws-meta">
        <span class="badge">
          <Icon name="folder-open" size={12} />
          {$t(countKey('workspaces_profiles_n', profiles.length, $locale), { n: String(profiles.length) })}
        </span>
        <span class="badge">
          <Icon name="globe" size={12} />
          {$t(countKey('workspaces_proxies_n', proxies.length, $locale), { n: String(proxies.length) })}
        </span>
        {#if runningCount > 0}
          <span class="badge badge-ok">
            <span class="dot"></span>
            {$t(countKey('workspaces_active_n', runningCount, $locale), { n: String(runningCount) })}
          </span>
        {/if}
      </div>

      <div class="ws-actions">
        {#if runningCount > 0}
          <button class="btn btn-ghost btn-sm" disabled={saving} onclick={stopAll}>
            <Icon name="square" size={12} />{$t('workspace_btn_stop_all')}
          </button>
        {/if}
        {#each drawers as drawer (drawer.id)}
          <button class="btn btn-ghost btn-sm" onclick={() => (drawerOpen[drawer.id] = true)} title={$t(drawer.title as TranslationKey)}>
            <Icon name={drawer.icon} size={12} />{$t((drawer.label ?? drawer.title) as TranslationKey)}
          </button>
        {/each}
        <button
          class="btn btn-ghost btn-sm"
          class:action-hidden={tab === 'proxies'}
          onclick={() => (showImportModal = true)}
        >
          <Icon name="download" size={12} />{$t('workspace_btn_import')}
        </button>
        <button
          class="btn btn-primary btn-sm"
          onclick={tab === 'proxies' ? openCreateProxy : () => (showCreatePanel = true)}
        >
          <Icon name="plus" size={12} />
          {tab === 'proxies' ? $t('proxies_add') : $t('workspace_btn_add_profile')}
        </button>
      </div>
    </div>

    {#if error}
      <div class="error-msg" style="margin-bottom:0.75rem">{error}</div>
    {/if}

    {#if fingerprintPrompt}
      <div class="fingerprint-banner">
        <div class="fingerprint-banner-icon">⚠</div>
        <div class="fingerprint-banner-body">
          <div class="fingerprint-banner-title">{$t('ssh_fingerprint_new_title')}</div>
          <div class="fingerprint-banner-fp">{fingerprintPrompt.fingerprint}</div>
          <div class="fingerprint-banner-hint">{$t('ssh_fingerprint_new_hint')}</div>
        </div>
        <div class="fingerprint-banner-actions">
          <button class="btn btn-primary btn-sm" onclick={trustFingerprint}>{$t('ssh_fingerprint_trust')}</button>
          <button class="btn btn-ghost btn-sm" onclick={() => fingerprintPrompt = null}>{$t('cancel')}</button>
        </div>
      </div>
    {/if}

    <!-- Tabs + View Switcher (segment controls per redesign) -->
    <div class="tabs-row">
      <div class="seg">
        <button class="seg-btn" class:active={tab === 'board'} onclick={() => (tab = 'board')}>
          <Icon name="columns" size={15} />{$t('workspace_tab_board')}
        </button>
        <button class="seg-btn" class:active={tab === 'proxies'} onclick={() => (tab = 'proxies')}>
          <Icon name="globe" size={15} />{$t('workspace_tab_proxies')}
          <span class="tab-count">{proxies.length}</span>
        </button>
        <button class="seg-btn" class:active={tab === 'topology'} onclick={() => (tab = 'topology')}>
          <Icon name="git-fork" size={15} />{$t('workspace_tab_topology')}
        </button>
      </div>

      <div class="seg view-switcher" class:hidden={tab !== 'board'}>
          <button
            class="seg-btn"
            class:active={viewMode === 'kanban'}
            onclick={() => setViewMode('kanban')}
            title={$t('ws_view_kanban')}
            aria-label={$t('ws_view_kanban')}
          >
            <Icon name="kanban" size={15} />
          </button>
          <button
            class="seg-btn"
            class:active={viewMode === 'table'}
            onclick={() => setViewMode('table')}
            title={$t('ws_view_table')}
            aria-label={$t('ws_view_table')}
          >
            <Icon name="list" size={15} />
          </button>
        </div>
    </div>

    <!-- Tab Content -->
    <div class="tab-content">
      {#if tab === 'board'}
        {#if viewMode === 'kanban'}
          <KanbanBoard
            bind:profiles
            bind:columns
            {proxies}
            {workspaceId}
            bind:runningProfiles
            onprofilechange={onProfileChange}
            oncolumnschange={onColumnsChange}
            onedit={(p) => (editingProfile = p)}
            onrawdata={(p) => (rawDataProfile = p)}
          />
        {:else}
          <TableView
            {profiles}
            proxies={proxiesStore.list}
            {columns}
            {runningProfiles}
            onSelect={(p) => (selectedProfile = p)}
            onEdit={(p) => (editingProfile = p)}
            onRefresh={onProfileChange}
          />
        {/if}

      {:else if tab === 'proxies'}
        <div class="proxies-tab">
          {#if proxies.length === 0}
            <div class="empty-state">
              <Icon name="globe" size={36} strokeWidth={1.5} />
              <p>{$t('proxies_empty')}</p>
              <button class="btn btn-primary btn-sm" onclick={openCreateProxy}>
                <Icon name="plus" size={13} />{$t('proxies_empty_add')}
              </button>
            </div>
          {:else}
            <div class="filter-bar">
              <div class="search-field wide">
                <Icon name="search" size={13} />
                <input
                  type="text"
                  bind:value={proxySearch}
                  placeholder={$t('proxies_search_placeholder')}
                />
                {#if proxySearch}
                  <button class="search-clear" onclick={() => (proxySearch = '')}><Icon name="x" size={11} /></button>
                {/if}
              </div>
              <span class="count-badge">{filteredProxies.length} / {proxies.length}</span>
            </div>

            {#if filteredProxies.length === 0}
              <div class="empty-state" style="padding:2rem">
                <Icon name="search" size={28} strokeWidth={1.5} />
                <p>{$t('proxies_not_found')}</p>
              </div>
            {:else}
              <div class="proxy-table-wrap">
                <table class="proxy-table">
                  <thead>
                    <tr>
                      <th class="col-num">#</th>
                      <th>{$t('proxy_col_name')}</th>
                      <th class="col-type">{$t('proxy_col_type')}</th>
                      <th>{$t('proxy_col_host')}</th>
                      <th class="col-status">{$t('proxy_col_status')}</th>
                      <th class="col-actions"></th>
                    </tr>
                  </thead>
                  <tbody>
                    {#each filteredProxies as proxy, i (proxy.id)}
                      {@const result = proxyCheckResults[proxy.id]}
                      <tr class="proxy-row" onclick={() => (proxyPanelProxy = proxy)}>
                        <td class="col-num">
                          <span class="row-num">{i + 1}</span>
                        </td>
                        <td class="col-name">
                          <span class="proxy-name">{proxy.name}</span>
                        </td>
                        <td class="col-type">
                          <span class="type-badge type-{proxy.proxy_type}">{proxy.proxy_type}</span>
                        </td>
                        <td><ProxyAddress {proxy} {result} /></td>
                        <td class="col-status">
                          <span class="status-badge status-{proxy.status}">{proxy.status}</span>
                        </td>
                        <td class="col-actions">
                          <div class="row-acts">
                            <button class="act-btn" title={$t('proxy_btn_check')} disabled={result?.checking} onclick={(e) => { e.stopPropagation(); checkProxy(proxy.id); }}>
                              <Icon name="refresh-cw" size={12} />
                            </button>
                            <button class="act-btn" title={$t('proxy_btn_edit')} onclick={(e) => { e.stopPropagation(); proxyPanelProxy = proxy; }}>
                              <Icon name="pencil" size={12} />
                            </button>
                            <button class="act-btn act-danger" title={$t('proxy_btn_delete')} onclick={(e) => { e.stopPropagation(); askDeleteProxy(proxy); }}>
                              <Icon name="trash-2" size={12} />
                            </button>
                          </div>
                        </td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            {/if}
          {/if}
        </div>

      {:else if tab === 'topology'}
        <!-- Full proxy list: profiles may reference proxies not tagged to this
             workspace (e.g. bulk-imported); the graph only draws referenced ones. -->
        <TopologyGraph {profiles} proxies={proxiesStore.list} {runningProfiles} />

      {:else if tab === 'notes'}
        <div class="notes-tab">
          <textarea
            class="notes-area"
            bind:value={notesValue}
            placeholder={$t('workspace_notes_placeholder')}
          ></textarea>
          <div class="notes-footer">
            <button class="btn btn-primary btn-sm" disabled={notesSaving} onclick={saveNotes}>
              {notesSaving ? '…' : $t('workspace_btn_save_notes')}
            </button>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

{#if showCreatePanel}
  <CreateProfilePanel
    {workspaceId}
    proxies={proxiesStore.list}
    onclose={() => (showCreatePanel = false)}
    oncreated={() => { showCreatePanel = false; onProfileChange(); }}
  />
{/if}

<ImportProfileModal
  {workspaceId}
  open={showImportModal}
  onclose={() => (showImportModal = false)}
  onimported={() => { showImportModal = false; onProfileChange(); }}
/>

{#if selectedProfile}
  <ProfileSidePanel
    profile={selectedProfile}
    proxy={proxiesStore.list.find((p) => p.id === selectedProfile?.proxy_id) ?? null}
    {workspaceId}
    {columns}
    isRunning={isSelectedRunning}
    onclose={() => (selectedProfile = null)}
    onchange={() => { selectedProfile = null; onProfileChange(); }}
    onsync={(updated) => {
      profilesStore.list = profilesStore.list.map((p) => p.id === updated.id ? updated : p);
      profiles = profilesStore.byWorkspace(workspaceId);
      selectedProfile = updated;
      if (updated.status === 'running') {
        runningProfiles = new Set([...runningProfiles, updated.id]);
      } else {
        const next = new Set(runningProfiles);
        next.delete(updated.id);
        runningProfiles = next;
      }
    }}
    onedit={(p) => { selectedProfile = null; editingProfile = p; }}
    onrawdata={(p) => { selectedProfile = null; rawDataProfile = p; }}
  />
{/if}

{#if rawDataProfile}
  <RawDataPanel
    profile={rawDataProfile}
    onclose={() => (rawDataProfile = null)}
  />
{/if}

{#if editingProfile}
  <EditProfilePanel
    profile={editingProfile}
    proxies={proxiesStore.list}
    onclose={() => (editingProfile = null)}
    onsaved={(updated) => {
      profilesStore.list = profilesStore.list.map((p) => p.id === updated.id ? updated : p);
      profiles = profilesStore.byWorkspace(workspaceId);
      if (selectedProfile?.id === updated.id) selectedProfile = updated;
      editingProfile = null;
    }}
  />
{/if}

{#if proxyPanelProxy !== undefined}
  <ProxyPanel
    proxy={proxyPanelProxy}
    workspaceId={workspaceId}
    onclose={() => (proxyPanelProxy = undefined)}
    onsaved={onProxyPanelSaved}
  />
{/if}

<Modal
  open={proxyDeleteModal.open}
  title={$t('proxy_btn_delete')}
  message={$t('proxy_confirm_delete', { name: proxyDeleteModal.name }) + proxyDeleteModal.usage}
  confirmLabel={$t('proxy_btn_delete')}
  cancelLabel={$t('proxy_btn_cancel')}
  variant="danger"
  onconfirm={confirmDeleteProxy}
  oncancel={() => (proxyDeleteModal = { open: false, id: '', name: '', usage: '' })}
/>

{#each drawers as drawer (drawer.id)}
  <drawer.component id={workspaceId} bind:open={drawerOpen[drawer.id]} />
{/each}

<style>
  /* width / centering / height come from global .page + .page--fill */
  .ws-page {
    gap: var(--sp-3);
    animation: vfade 0.25s ease;
  }

  .centered { text-align: center; color: var(--text-2); padding: var(--sp-12); }

  .ws-header {
    display: flex; flex-wrap: wrap; align-items: center; gap: var(--sp-3);
    padding-bottom: 0.625rem;
  }

  .breadcrumb {
    display: flex; align-items: center; gap: 12px; font-size: var(--fs-base); color: var(--text-2);
  }
  .crumb-sep { color: var(--text-3); }

  .back-link {
    display: flex; align-items: center; gap: 7px;
    color: var(--text-2); text-decoration: none; transition: color 0.15s;
    font-size: var(--fs-base);
  }
  .back-link:hover { color: var(--text); }

  /* Workspace name in breadcrumb — accent per redesign */
  .ws-name { font-weight: var(--fw-bold); font-size: 1.15rem; color: var(--accent-text); }

  .ws-meta { display: flex; align-items: center; gap: 0.4rem; flex: 1; }

  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--success); }

  .ws-actions { display: flex; gap: 0.5rem; flex-wrap: wrap; margin-left: auto; }
  .action-hidden { visibility: hidden; pointer-events: none; }

  /* Tabs row: segment control + view switcher (both .seg primitives) */
  .tabs-row {
    display: flex; align-items: center; justify-content: space-between;
    flex-shrink: 0;
  }

  .view-switcher.hidden { visibility: hidden; pointer-events: none; }

  /* Tab Content */
  .tab-content {
    flex: 1; min-height: 0; display: flex; flex-direction: column; overflow: hidden;
  }

  /* Proxies Tab */
  .proxies-tab { display: flex; flex-direction: column; gap: 0.625rem; flex: 1; min-height: 0; }

  /* панель фильтров — единые примитивы .filter-bar/.search-field/.count-badge из base.css */
  /* The proxies search names four fields: the width of the SSH keys search, so it is read whole. */
  .search-field.wide { width: 300px; }

  .proxy-table-wrap {
    flex: 1; min-height: 0; overflow-y: auto;
    border: 1px solid var(--border); border-radius: var(--radius-lg);
    background: var(--surface);
  }

  .proxy-table { width: 100%; border-collapse: collapse; font-size: var(--fs-sm); }
  .proxy-table thead {
    position: sticky; top: 0; z-index: 1;
    background: var(--surface); border-bottom: 1px solid var(--border);
  }
  .proxy-table th {
    padding: var(--sp-3) var(--sp-4); text-align: left;
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--text-3);
    text-transform: uppercase; letter-spacing: 0.7px; white-space: nowrap;
  }
  .proxy-table td { padding: var(--sp-3) var(--sp-4); vertical-align: middle; }
  .proxy-row { border-bottom: 1px solid var(--surface-2); cursor: pointer; transition: background 0.12s; }
  .proxy-row:last-child { border-bottom: none; }
  .proxy-row:hover { background: var(--surface-row-hover); }

  .col-num { width: 44px; }
  .col-type { width: 90px; }
  .col-status { width: 110px; }
  .col-actions { width: 120px; text-align: right; }

  .row-num { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-3); }
  .proxy-name { font-weight: var(--fw-semibold); display: block; }

  /* Proxy type badges: http → blue tint, socks5 → purple tint (design) */
  .type-badge {
    font-family: var(--font-mono);
    font-size: var(--fs-xs); font-weight: var(--fw-semibold);
    padding: 4px 12px; border-radius: var(--radius-sm);
    border: none; background: var(--surface-2); color: var(--text-2);
  }
  .type-http, .type-https { background: color-mix(in srgb, var(--cat-blue) 14%, transparent); color: var(--cat-blue); }
  .type-socks5 { background: var(--accent-tint); color: var(--accent-text-2); }
  .type-tor { background: var(--accent-tint); color: var(--accent-text-2); }

  .status-badge {
    display: inline-flex; align-items: center; gap: 7px;
    font-size: 0.72rem; font-weight: var(--fw-bold); text-transform: uppercase; letter-spacing: 0.4px;
    padding: 4px 11px; border-radius: var(--radius-sm);
  }
  .status-badge::before { content: ''; width: 6px; height: 6px; border-radius: 50%; background: currentColor; }
  .status-active { background: var(--success-bg); color: var(--success-text); }
  .status-failed { background: var(--danger-bg); color: var(--danger-text); }
  .status-unknown { background: var(--surface-2); color: var(--text-2); }

  .row-acts { display: flex; gap: 5px; justify-content: flex-end; }
  .act-btn {
    display: inline-flex; align-items: center; justify-content: center;
    width: 32px; height: 32px;
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius-sm);
    color: var(--text-soft); cursor: pointer; padding: 0; transition: all 0.15s;
  }
  .act-btn:hover { border-color: var(--border-2); color: var(--text); }
  .act-btn:first-child:hover { border-color: var(--accent); color: var(--accent-text); } /* check/refresh → accent */
  .act-btn:disabled { opacity: 0.35; cursor: not-allowed; }
  .act-danger:hover { background: var(--danger-bg) !important; border-color: var(--danger-border) !important; color: var(--danger-text) !important; }

  /* Notes Tab */
  .notes-tab { display: flex; flex-direction: column; gap: var(--sp-3); flex: 1; }
  .notes-area { flex: 1; resize: none; min-height: 200px; font-size: var(--fs-sm); line-height: 1.6; }
  .notes-footer { display: flex; justify-content: flex-end; }

  .fingerprint-banner {
    display: flex; align-items: flex-start; gap: var(--sp-3);
    background: color-mix(in srgb, var(--color-warning) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-warning) 40%, transparent);
    border-radius: var(--radius); padding: 0.875rem var(--sp-4); margin-bottom: var(--sp-3);
  }
  .fingerprint-banner-icon { font-size: var(--fs-lg); flex-shrink: 0; margin-top: 0.1rem; }
  .fingerprint-banner-body { flex: 1; display: flex; flex-direction: column; gap: var(--sp-1); }
  .fingerprint-banner-title { font-weight: 700; font-size: var(--fs-base); }
  .fingerprint-banner-fp {
    font-family: var(--font-mono); font-size: var(--fs-sm); color: var(--text-2);
    word-break: break-all;
  }
  .fingerprint-banner-hint { font-size: var(--fs-sm); color: var(--text-2); }
  .fingerprint-banner-actions { display: flex; gap: var(--sp-2); flex-shrink: 0; align-items: flex-start; }
</style>

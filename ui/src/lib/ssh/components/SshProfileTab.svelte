<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { api } from '$lib/ssh/api';
  import { directory } from '$lib/core/directory';
  import { explainError } from '$lib/ssh/tor-error';
  import type { SshConnection } from '$lib/ssh/types';
  import { sshStore } from '$lib/ssh/store/ssh.svelte';
  import SSHConnectionsList from './SSHConnectionsList.svelte';
  import SSHConnectionForm from './SSHConnectionForm.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import Suggest, { type SuggestItem } from '$lib/core/ui/Suggest.svelte';
  import { t } from '$lib/core/i18n';

  interface Props {
    profileId: string;
    workspaceId: string;
  }

  let { profileId, workspaceId }: Props = $props();

  let allConns = $state<SshConnection[]>([]);
  let loading = $state(true);
  let showPicker = $state(false);
  let showForm = $state(false);
  let editConn = $state<SshConnection | null>(null);
  let pickerSearch = $state('');
  let workspaceNames = $state<Record<string, string>>({});
  let connForm = $state<ReturnType<typeof SSHConnectionForm>>();
  let connSaving = $state(false);

  $effect(() => {
    loadAll();
    // Workspace names come through the catalog of entities (the browser module owns them).
    directory.get('workspace')?.ensureLoaded().then(() => {
      const map: Record<string, string> = {};
      directory.list('workspace').forEach((w) => (map[w.id] = w.name));
      workspaceNames = map;
    });
  });

  async function loadAll() {
    loading = true;
    try {
      allConns = await api.ssh.connectionList();
    } finally {
      loading = false;
    }
  }

  let linked = $derived(allConns.filter((c) => c.profile_ids.includes(profileId)));

  let pickerConns = $derived(
    allConns
      .filter((c) => !c.profile_ids.includes(profileId))
      .filter((c) => {
        if (!pickerSearch.trim()) return true;
        const q = pickerSearch.toLowerCase();
        return (
          c.name.toLowerCase().includes(q) ||
          c.host.toLowerCase().includes(q) ||
          c.username.toLowerCase().includes(q)
        );
      })
  );

  let searchEl = $state<HTMLInputElement | null>(null);
  let searchRow = $state<HTMLDivElement | null>(null);
  let pickerList = $state<Suggest | null>(null);
  /** The list shows while the search has the focus. */
  let searching = $state(false);
  const pickerItems = $derived<SuggestItem[]>(
    pickerConns.map((c) => ({ key: c.id, text: c.name, icon: 'terminal', hint: `${c.username}@${c.host}:${c.port}` })),
  );
  $effect(() => { if (showPicker) searchEl?.focus(); });

  async function link(conn: SshConnection) {
    const updated = await api.ssh.connectionUpdate(conn.id, {
      profile_ids: [...conn.profile_ids, profileId],
    });
    applyUpdate(updated);
  }

  async function unlink(conn: SshConnection) {
    const updated = await api.ssh.connectionUpdate(conn.id, {
      profile_ids: conn.profile_ids.filter((id) => id !== profileId),
    });
    applyUpdate(updated);
  }

  /** A connection saved here: in place if it was in the list, at its end if it was just made. */
  function applyUpdate(updated: SshConnection) {
    const put = (list: SshConnection[]) =>
      list.some((c) => c.id === updated.id) ? list.map((c) => (c.id === updated.id ? updated : c)) : [...list, updated];
    allConns = put(allConns);
    sshStore.connections = put(sshStore.connections);
  }

  function handleEdit(conn: SshConnection) {
    editConn = conn;
    showForm = true;
    showPicker = false;
  }

  function closeForm() {
    showForm = false;
    editConn = null;
  }

  function handleFormSave(conn: SshConnection) {
    applyUpdate(conn);
    closeForm();
  }

  let connectError = $state('');

  function handleConnect(conn: SshConnection) {
    connectError = '';
    sshStore.connect(conn.id).catch((e: unknown) => {
      connectError = explainError(e, $t);
    });
  }
</script>

<div class="ssh-tab">
  {#if loading}
    <div class="loading"><Icon name="loader" size={14} /></div>
  {:else}
    <div class="tab-header">
      <span class="count">{$t('ssh_profile_linked_count', { n: String(linked.length) })}</span>
      <div class="header-actions">
        <button class="btn btn-sm btn-ghost" onclick={() => { showPicker = !showPicker; pickerSearch = ''; }}>
          <Icon name={showPicker ? 'x' : 'link'} size={12} />
          {showPicker ? $t('common_cancel') : $t('ssh_btn_link')}
        </button>
        <button class="btn btn-sm btn-primary" onclick={() => { showForm = true; editConn = null; }}>
          <Icon name="plus" size={12} /> {$t('ssh_btn_new')}
        </button>
      </div>
    </div>

    {#if showPicker}
      <div class="search-row" bind:this={searchRow}>
        <Icon name="search" size={13} />
        <input
          class="search-input"
          type="text"
          bind:this={searchEl}
          bind:value={pickerSearch}
          placeholder={$t('ssh_search_placeholder')}
          onfocus={() => (searching = true)}
          onblur={() => (searching = false)}
          oninput={() => (searching = true)}
          onkeydown={(e) => { pickerList?.keydown(e); }}
        />
      </div>
      {#if searching}
        <Suggest
          bind:this={pickerList}
          items={pickerItems}
          anchor={searchRow}
          label={$t('ssh_btn_link')}
          empty={pickerSearch ? $t('common_nothing_found') : $t('ssh_picker_all_linked')}
          onpick={(item) => { const conn = pickerConns.find((c) => c.id === item.key); if (conn) void link(conn); }}
          onclose={() => { showPicker = false; pickerSearch = ''; }}
        />
      {/if}
    {/if}

    <SSHConnectionsList
      connections={linked}
      {workspaceNames}
      onEdit={handleEdit}
      onConnect={handleConnect}
      onUnlink={unlink}
      onOpenTerminal={() => {}}
    />

    {#if connectError}
      <div class="connect-error">{connectError}</div>
    {/if}

    <button class="open-panel-btn" onclick={() => sshStore.openPanel()}>
      <Icon name="settings" size={12} /> {$t('ssh_btn_manage_all')}
    </button>
  {/if}
</div>

<Drawer
  open={showForm}
  width="var(--drawer-w-md)"
  title={editConn ? $t('ssh_form_edit') : $t('ssh_form_new')}
  onclose={closeForm}
>
  {#if showForm}
    <SSHConnectionForm
      bind:this={connForm}
      bind:saving={connSaving}
      connection={editConn}
      defaultWorkspaceId={workspaceId}
      defaultProfileId={profileId}
      onSave={handleFormSave}
    />
  {/if}
  {#snippet footer()}
    <button class="btn btn-ghost" onclick={closeForm}>{$t('common_cancel')}</button>
    <button class="btn btn-primary" onclick={() => connForm?.submit()} disabled={connSaving}>
      {#if connSaving}<Icon name="loader" size={18} />{/if}
      {editConn ? $t('common_save') : $t('common_create')}
    </button>
  {/snippet}
</Drawer>

<style>
  .ssh-tab { display: flex; flex-direction: column; gap: var(--sp-3); padding: var(--sp-2) 0; min-height: 100%; }

  .tab-header { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); }
  .count { font-size: var(--fs-sm); color: var(--text-faint); }
  .header-actions { display: flex; gap: 8px; }
  /* Кнопки шапки вкладки: компактные 34px по дизайну вторичных действий */
  .header-actions :global(.btn) {
    height: 34px; padding: 0 13px; font-size: var(--fs-sm); border-radius: var(--radius);
  }

  /* The search of the connections to link: a field; they are the list of suggestions under it. */
  .search-row {
    display: flex; align-items: center; gap: var(--sp-2);
    min-height: var(--control-h); padding: 0 var(--sp-3);
    background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-field);
    color: var(--text-2);
  }
  .search-row:focus-within { border-color: var(--accent-border); }
  .search-input {
    flex: 1; min-width: 0; height: auto; padding: 0; background: none; border: none; outline: none; box-shadow: none;
    color: var(--text); font-size: var(--fs-sm); font-family: inherit;
  }
  .search-input:focus { box-shadow: none; }
  .search-input::placeholder { color: var(--text-3); }

  /* «Управление всеми» — вторичная 46px кнопка, прижата к низу вкладки
     (тот же паттерн, что стек действий на вкладке Info) */
  .open-panel-btn {
    display: flex; align-items: center; justify-content: center; gap: 9px;
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius-field);
    height: 46px; padding: 0 14px; cursor: pointer; color: var(--text-body);
    font-size: var(--fs-base); font-weight: var(--fw-semibold); font-family: inherit; width: 100%;
    transition: all var(--dur-fast); margin-top: auto; flex-shrink: 0;
  }
  .open-panel-btn:hover { background: var(--surface-hover); border-color: var(--border-2); color: var(--text); }

  .connect-error {
    background: var(--danger-bg);
    color: var(--danger-text);
    border-radius: var(--radius-md);
    padding: 0.35rem 0.6rem;
    font-size: var(--fs-xs);
    word-break: break-all;
  }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { api } from '$lib/ssh/api';
  import { directory } from '$lib/core/directory';
  import type { SshConnection } from '$lib/ssh/types';
  import { sshStore } from '$lib/ssh/store/ssh.svelte';
  import SSHConnectionsList from './SSHConnectionsList.svelte';
  import SSHConnectionForm from './SSHConnectionForm.svelte';
  import Icon from '$lib/core/Icon.svelte';
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

  function applyUpdate(updated: SshConnection) {
    allConns = allConns.map((c) => (c.id === updated.id ? updated : c));
    sshStore.connections = sshStore.connections.map((c) => (c.id === updated.id ? updated : c));
  }

  function handleEdit(conn: SshConnection) {
    editConn = conn;
    showForm = true;
    showPicker = false;
  }

  function handleFormSave(conn: SshConnection) {
    applyUpdate(conn);
    showForm = false;
    editConn = null;
  }

  let connectError = $state('');

  function handleConnect(conn: SshConnection) {
    connectError = '';
    sshStore.connect(conn.id).catch((e: unknown) => {
      connectError = String(e);
    });
  }
</script>

<div class="ssh-tab">
  {#if loading}
    <div class="loading"><Icon name="loader" size={14} /></div>
  {:else if showForm}
    <div class="form-header">
      <button class="back-btn" onclick={() => { showForm = false; editConn = null; }}>
        <Icon name="arrow-left" size={13} /> {$t('ssh_btn_cancel')}
      </button>
      <span class="form-title">{editConn ? $t('ssh_form_edit') : $t('ssh_form_new')}</span>
    </div>
    <SSHConnectionForm
      connection={editConn}
      defaultWorkspaceId={workspaceId}
      defaultProfileId={profileId}
      onSave={handleFormSave}
      onCancel={() => { showForm = false; editConn = null; }}
    />
  {:else}
    <div class="tab-header">
      <span class="count">{$t('ssh_profile_linked_count', { n: String(linked.length) })}</span>
      <div class="header-actions">
        <button class="btn btn-sm btn-ghost" onclick={() => { showPicker = !showPicker; pickerSearch = ''; }}>
          <Icon name={showPicker ? 'x' : 'link'} size={12} />
          {showPicker ? $t('ssh_btn_cancel') : $t('ssh_btn_link')}
        </button>
        <button class="btn btn-sm btn-primary" onclick={() => { showForm = true; editConn = null; }}>
          <Icon name="plus" size={12} /> {$t('ssh_btn_new')}
        </button>
      </div>
    </div>

    {#if showPicker}
      <div class="picker">
        <div class="search-row">
          <Icon name="search" size={13} />
          <input
            class="search-input"
            type="text"
            bind:value={pickerSearch}
            placeholder={$t('ssh_search_placeholder')}
          />
        </div>
        <div class="picker-list">
          {#if pickerConns.length === 0}
            <div class="empty-state">
              <p>{pickerSearch ? $t('ssh_search_empty') : $t('ssh_picker_all_linked')}</p>
            </div>
          {/if}
          {#each pickerConns as conn (conn.id)}
            <button class="picker-item" onclick={() => link(conn)}>
              <div class="pi-info">
                <span class="pi-name">{conn.name}</span>
                <span class="pi-sub">{conn.username}@{conn.host}:{conn.port}</span>
              </div>
              <Icon name="plus" size={12} />
            </button>
          {/each}
        </div>
      </div>
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

<style>
  .ssh-tab { display: flex; flex-direction: column; gap: var(--sp-3); padding: var(--sp-2) 0; min-height: 100%; }

  .tab-header { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); }
  .count { font-size: 0.78rem; color: var(--text-faint); }
  .header-actions { display: flex; gap: 8px; }
  /* Кнопки шапки вкладки: компактные 34px по дизайну вторичных действий */
  .header-actions :global(.btn) {
    height: 34px; padding: 0 13px; font-size: 0.82rem; border-radius: 9px;
  }

  .form-header { display: flex; align-items: center; gap: var(--sp-2); margin-bottom: var(--sp-1); }
  .back-btn {
    display: flex; align-items: center; gap: 0.3rem;
    background: none; border: none; cursor: pointer;
    color: var(--text-2); font-size: var(--fs-sm); font-family: inherit; padding: 0.2rem 0;
  }
  .back-btn:hover { color: var(--text); }
  .form-title { font-size: var(--fs-sm); font-weight: var(--fw-medium); color: var(--text); }

  .picker {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow: hidden;
  }
  .search-row {
    display: flex; align-items: center; gap: 0.35rem;
    padding: 0.35rem var(--sp-2);
    border-bottom: 1px solid var(--border);
    color: var(--text-2);
  }
  .search-input {
    flex: 1; background: none; border: none; outline: none;
    color: var(--text); font-size: var(--fs-sm); font-family: inherit;
  }
  .search-input::placeholder { color: var(--text-3); }
  .picker-list { max-height: 160px; overflow-y: auto; }
  .picker-item {
    width: 100%; display: flex; align-items: center; justify-content: space-between;
    padding: 0.35rem 0.6rem; background: none; border: none; cursor: pointer;
    text-align: left; gap: var(--sp-2); transition: background 0.12s;
  }
  .picker-item:hover { background: var(--surface-hover); }
  .pi-info { display: flex; flex-direction: column; gap: 1px; }
  .pi-name { font-size: var(--fs-sm); color: var(--text); }
  .pi-sub { font-size: var(--fs-2xs); color: var(--text-2); font-family: var(--font-mono); }

  /* «Управление всеми» — вторичная 46px кнопка, прижата к низу вкладки
     (тот же паттерн, что стек действий на вкладке Info) */
  .open-panel-btn {
    display: flex; align-items: center; justify-content: center; gap: 9px;
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius-field);
    height: 46px; padding: 0 14px; cursor: pointer; color: var(--text-body);
    font-size: 0.9rem; font-weight: var(--fw-semibold); font-family: inherit; width: 100%;
    transition: all var(--dur-fast); margin-top: auto; flex-shrink: 0;
  }
  .open-panel-btn:hover { background: var(--surface-hover); border-color: var(--border-2); color: var(--text); }

  .connect-error {
    background: var(--danger-bg);
    color: var(--danger-text);
    border-radius: var(--radius-sm);
    padding: 0.35rem 0.6rem;
    font-size: var(--fs-xs);
    word-break: break-all;
  }
</style>

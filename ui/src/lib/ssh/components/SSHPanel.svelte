<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { sshStore } from '$lib/ssh/store/ssh.svelte';
  import type { SshConnection } from '$lib/ssh/types';
  import { api } from '$lib/ssh/api';
  import { directory } from '$lib/core/directory';
  import { explainError } from '$lib/ssh/tor-error';
  import Icon from '$lib/core/Icon.svelte';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import SSHConnectionsList from './SSHConnectionsList.svelte';
  import SSHConnectionForm from './SSHConnectionForm.svelte';
  import { t } from '$lib/core/i18n';

  interface Props {
    open?: boolean;
    /** 'global' | 'workspace' | 'profile' */
    context?: string;
    workspaceId?: string | null;
    profileId?: string | null;
  }

  let {
    open = $bindable(false),
    context = 'global',
    workspaceId = null,
    profileId = null,
  }: Props = $props();

  let editConn = $state<SshConnection | null>(null);
  let showForm = $state(false);
  let showAll = $state(false);
  let connecting = $state<string | null>(null);
  let connectError = $state('');
  let workspaceNames = $state<Record<string, string>>({});

  onMount(() => {
    sshStore.ensureLoaded();
    // Workspace names come through the catalog of entities (the browser module owns them).
    directory.get('workspace')?.ensureLoaded().then(() => {
      const map: Record<string, string> = {};
      directory.list('workspace').forEach((w) => (map[w.id] = w.name));
      workspaceNames = map;
    });
  });

  $effect(() => {
    if (open) {
      untrack(() => {
        showAll = context === 'global';
        sshStore.ensureLoaded();
      });
    }
  });

  // Filtering: in workspace context show global (no ws links) OR linked to this ws
  // In profile context — only linked to this profile
  // In global context — all
  let filteredConnections = $derived(
    showAll
      ? sshStore.connections
      : context === 'profile' && profileId
        ? sshStore.connections.filter((c) => c.profile_ids.includes(profileId!))
        : context === 'workspace' && workspaceId
          ? sshStore.connections.filter(
              (c) => c.workspace_ids.length === 0 || c.workspace_ids.includes(workspaceId!)
            )
          : sshStore.connections
  );

  function handleEdit(conn: SshConnection) {
    editConn = conn;
    showForm = true;
  }

  function handleNew() {
    editConn = null;
    showForm = true;
  }

  function handleFormSave(conn: SshConnection) {
    const idx = sshStore.connections.findIndex((c) => c.id === conn.id);
    if (idx >= 0) {
      sshStore.connections[idx] = conn;
    } else {
      sshStore.connections = [...sshStore.connections, conn];
    }
    showForm = false;
    editConn = null;
  }

  async function handleConnect(conn: SshConnection) {
    connecting = conn.id;
    connectError = '';
    try {
      await sshStore.connect(conn.id);
      open = false;
    } catch (e: unknown) {
      connectError = explainError(e, $t);
    } finally {
      connecting = null;
    }
  }

  function close() {
    open = false;
    showForm = false;
    editConn = null;
  }

  let contextLabel = $derived(
    context === 'profile' ? $t('ssh_ctx_profile')
    : context === 'workspace' ? $t('ssh_ctx_workspace')
    : $t('ssh_ctx_global')
  );
</script>

<Drawer open={open} width="480px" title={$t('ssh_title')} onclose={close}>
  {#snippet actions()}
    {#if context !== 'global'}
      <div class="tab-bar">
        <button class="tab" class:active={!showAll} onclick={() => (showAll = false)}>
          {contextLabel}
        </button>
        <button class="tab" class:active={showAll} onclick={() => (showAll = true)}>
          {$t('ssh_ctx_all')}
        </button>
      </div>
    {/if}
  {/snippet}

  <div class="ssh-body">
  {#if showForm}
    <div class="form-header">
      <button class="btn btn-ghost btn-sm" onclick={() => { showForm = false; editConn = null; }}>
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
    {#if connectError}
      <div class="error-msg">{connectError}</div>
    {/if}

    <div class="list-header">
      <span class="list-count">{$t('ssh_connections_count', { n: String(filteredConnections.length) })}</span>
      <button class="btn btn-primary btn-sm" onclick={handleNew}>
        <Icon name="plus" size={13} /> {$t('ssh_btn_new')}
      </button>
    </div>

    <SSHConnectionsList
      connections={filteredConnections}
      {workspaceNames}
      onEdit={handleEdit}
      onConnect={handleConnect}
      onOpenTerminal={close}
    />
  {/if}
  </div>
</Drawer>

<style>
  .ssh-body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .form-header {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-bottom: var(--sp-1);
  }
  .form-title { font-size: var(--fs-sm); font-weight: var(--fw-medium); color: var(--text); }
  .list-header { display: flex; align-items: center; justify-content: space-between; }
  .list-count { font-size: var(--fs-sm); color: var(--text-2); }
</style>

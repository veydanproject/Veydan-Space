<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { SshConnection, SshSessionInfo } from '$lib/ssh/types';
  import Pill from '$lib/core/ui/Pill.svelte';
  import { api } from '$lib/ssh/api';
  import { sshStore } from '$lib/ssh/store/ssh.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { ask, confirmTitle } from '$lib/core/ui/confirm.svelte';
  import { t, locale } from '$lib/core/i18n';
  import { formatDate } from '$lib/core/utils';

  interface Props {
    connections: SshConnection[];
    workspaceNames?: Record<string, string>;
    onEdit: (conn: SshConnection) => void;
    onConnect: (conn: SshConnection) => void;
    onOpenTerminal?: () => void;
    onUnlink?: (conn: SshConnection) => void;
  }

  let { connections, workspaceNames = {}, onEdit, onConnect, onOpenTerminal, onUnlink }: Props = $props();

  let search = $state('');
  let filtered = $derived(
    search.trim()
      ? connections.filter((c) => {
          const q = search.toLowerCase();
          return (
            c.name.toLowerCase().includes(q) ||
            c.host.toLowerCase().includes(q) ||
            c.username.toLowerCase().includes(q)
          );
        })
      : connections
  );

  function sessionForConn(id: string): SshSessionInfo | undefined {
    return sshStore.sessions.find((s) => s.connection_id === id);
  }

  function statusColor(status: string): string {
    if (status === 'connected') return 'var(--success)';
    if (status === 'connecting') return 'var(--warn-text)';
    return 'var(--danger)';
  }

  async function doDelete(conn: { id: string; name: string }) {
    const yes = await ask({
      title: confirmTitle($t('ssh_delete_title')),
      message: $t('ssh_delete_message', { name: conn.name }),
      confirmLabel: $t('common_delete'),
      variant: 'danger',
      icon: 'trash-2',
    });
    if (!yes) return;
    try {
      await api.ssh.connectionDelete(conn.id);
      sshStore.connections = sshStore.connections.filter((c) => c.id !== conn.id);
    } catch {}
  }
</script>

<div class="list-wrap">
  <div class="search-row">
    <Icon name="search" size={14} />
    <input
      class="search-input"
      type="text"
      bind:value={search}
      placeholder={$t('ssh_search_placeholder')}
    />
    {#if search}
      <button class="clear-btn" onclick={() => (search = '')}>
        <Icon name="x" size={12} />
      </button>
    {/if}
  </div>

  <div class="list">
    {#if filtered.length === 0}
      <div class="empty-state">
        {#if search}
          <span class="empty-icon"><Icon name="search" size={22} /></span>
          <p>{$t('common_nothing_found')}</p>
          <span>{$t('common_nothing_found_hint', { q: search.trim() })}</span>
        {:else}
          <span class="empty-icon"><Icon name="terminal" size={22} /></span>
          <p>{$t('ssh_list_empty')}</p>
        {/if}
      </div>
    {/if}
    {#each filtered as conn (conn.id)}
      {@const session = sessionForConn(conn.id)}
      <div class="item">
        <div class="item-main" role="button" tabindex="0" onclick={() => onEdit(conn)} onkeydown={(e) => e.key === 'Enter' && onEdit(conn)}>
          <div class="item-icon">
            <Icon name="terminal" size={15} />
          </div>
          <div class="item-info">
            <div class="item-name">
              {conn.name}
              {#if session}
                <span class="status-dot" style="background:{statusColor(session.status)}" title={session.status}></span>
              {/if}
            </div>
            <div class="item-sub">{conn.username}@{conn.host}:{conn.port}</div>
            <div class="item-badges">
              {#if conn.workspace_ids.length === 0}
                <Pill text={$t('ssh_ctx_global')} />
              {:else}
                {#each conn.workspace_ids.slice(0, 2) as wid}
                  <Pill tone="accent" text={workspaceNames[wid] ?? wid.slice(0, 6)} />
                {/each}
                {#if conn.workspace_ids.length > 2}
                  <Pill text="+{conn.workspace_ids.length - 2}" />
                {/if}
              {/if}
              {#if conn.proxy_id && conn.proxy_name}
                <Pill tone="accent" text={conn.proxy_name} />
              {:else if conn.proxy_id}
                <!-- Proxy row is gone: the connect fails closed rather than going
                     out directly, so say that instead of claiming a live proxy. -->
                <Pill tone="bad" title={$t('ssh_proxy_missing_hint')} text={$t('ssh_proxy_missing')} />
              {/if}
              {#if conn.requires_2fa}
                <Pill tone="accent" text="2FA" />
              {/if}
              {#if conn.last_connected_at}
                <span class="last">{formatDate(conn.last_connected_at, $locale)}</span>
              {/if}
            </div>
          </div>
        </div>
        <div class="item-actions">
          {#if session && (session.status === 'connected' || session.status === 'connecting')}
            <button
              class="icon-btn success"
              title={$t('ssh_btn_open_terminal')}
              aria-label={$t('ssh_btn_open_terminal')}
              onclick={() => {
                sshStore.activeTerminalId = session.session_id;
                onOpenTerminal?.();
              }}
            >
              <Icon name="terminal" size={13} />
            </button>
            <button
              class="icon-btn danger-soft"
              title={$t('ssh_btn_disconnect')}
              aria-label={$t('ssh_btn_disconnect')}
              onclick={async () => {
                await sshStore.disconnect(session.session_id);
                await sshStore.removeSession(session.session_id);
              }}
            >
              <Icon name="plug-connected-x" size={13} />
            </button>
          {:else}
            <button class="icon-btn success" title={$t('ssh_btn_connect')} aria-label={$t('ssh_btn_connect')} onclick={() => onConnect(conn)}>
              <Icon name="play" size={13} />
            </button>
          {/if}
          <button
              class="icon-btn" title={$t('common_edit')} aria-label={$t('common_edit')} onclick={() => onEdit(conn)}>
              <Icon name="pencil" size={13} />
            </button>
            {#if onUnlink}
              <button
                class="icon-btn danger-soft"
                title={$t('ssh_btn_unlink')}
                onclick={() => onUnlink!(conn)}
              >
                <Icon name="minus" size={13} />
              </button>
            {:else}
              <button
                class="icon-btn danger-soft"
                title={$t('common_delete')}
                aria-label={$t('common_delete')}
                onclick={() => doDelete(conn)}
              >
                <Icon name="trash-2" size={13} />
              </button>
            {/if}
        </div>
      </div>
    {/each}
  </div>
</div>

<style>
  .list-wrap { display: flex; flex-direction: column; gap: var(--sp-2); }

  .search-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-pill);
    height: var(--control-h);
    padding: 0 var(--sp-3);
    color: var(--text-2);
  }
  .search-row:focus-within { border-color: var(--accent-border); }
  .search-input {
    flex: 1;
    background: none;
    border: none;
    outline: none;
    box-shadow: none;
    color: var(--text);
    font-size: var(--fs-sm);
    font-family: inherit;
  }
  .search-input::placeholder { color: var(--text-3); }
  .clear-btn {
    background: none; border: none; cursor: pointer;
    color: var(--text-3); padding: 0; display: flex; align-items: center;
  }
  .clear-btn:hover { color: var(--text); }

  .list { display: flex; flex-direction: column; gap: 0.3rem; }

  .item {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 0.875rem;
    transition: border-color var(--dur-fast);
  }
  .item:hover { border-color: var(--border-2); }
  .item-main {
    display: flex;
    align-items: flex-start;
    gap: 0.55rem;
    flex: 1;
    cursor: pointer;
    min-width: 0;
  }
  .item-icon { color: var(--text-soft); flex-shrink: 0; padding-top: 2px; }
  .item-info { flex: 1; min-width: 0; }
  .item-name {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--fs-sm);
    font-weight: var(--fw-semibold);
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status-dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; }
  .item-sub { font-size: var(--fs-xs); color: var(--text-2); font-family: var(--font-mono); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin-top: 1px; }
  .item-badges { display: flex; flex-wrap: wrap; gap: 0.2rem; margin-top: 3px; align-items: center; }
  .item-badges :global(.badge) { font-family: var(--font-mono); font-size: var(--fs-xs); padding: 0.15rem 0.45rem; }
  .last { font-size: var(--fs-xs); color: var(--text-3); }
  .item-actions { display: flex; gap: 0.2rem; flex-shrink: 0; }
</style>

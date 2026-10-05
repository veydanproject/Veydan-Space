<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t, locale } from '$lib/core/i18n';
  import { formatDate, formatTime } from '$lib/core/utils';
  import type { SshConnection } from '$lib/ssh/types';
  import Icon from '$lib/core/Icon.svelte';
  import Modal from '$lib/core/ui/Modal.svelte';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import SSHConnectionForm from '$lib/ssh/components/SSHConnectionForm.svelte';
  import SshKeysTab from '$lib/ssh/components/keys/SshKeysTab.svelte';
  import { sshStore } from '$lib/ssh/store/ssh.svelte';
  import { directory } from '$lib/core/directory';
  import { api } from '$lib/ssh/api';
  import { explainError } from '$lib/ssh/tor-error';

  const PAGE_SIZE = 20;

  let loading = $state(false);
  let error = $state('');
  let activeTab = $state<'connections' | 'keys'>('connections');

  let search = $state('');
  let filterAuth = $state('all');
  let page = $state(0);

  let panelConn = $state<SshConnection | null | undefined>(undefined);
  let deleteModal = $state({ open: false, id: '', name: '' });
  let connectingId = $state<string | null>(null);

  onMount(async () => {
    loading = true;
    try { await Promise.all([sshStore.ensureLoaded(), directory.get('proxy')?.ensureLoaded()]); }
    catch (e) { error = explainError(e, $t); }
    finally { loading = false; }
  });

  let filtered = $derived(sshStore.connections.filter((c) => {
    if (filterAuth !== 'all' && c.auth_type !== filterAuth) return false;
    if (search.trim()) {
      const q = search.toLowerCase();
      return (
        c.name.toLowerCase().includes(q) ||
        c.host.toLowerCase().includes(q) ||
        c.username.toLowerCase().includes(q)
      );
    }
    return true;
  }));

  let totalPages = $derived(Math.max(1, Math.ceil(filtered.length / PAGE_SIZE)));
  let currentPage = $derived(Math.min(page, totalPages - 1));
  let pageItems = $derived(filtered.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE));

  $effect(() => {
    search; filterAuth;
    page = 0;
  });

  function connStatus(id: string): 'connected' | 'connecting' | 'idle' {
    const sessions = sshStore.sessionsForConnection(id);
    if (sessions.some((s) => s.status === 'connected')) return 'connected';
    if (sessions.some((s) => s.status === 'connecting')) return 'connecting';
    return 'idle';
  }

  async function handleConnect(conn: SshConnection) {
    connectingId = conn.id;
    try {
      await sshStore.connect(conn.id);
    } catch (e) {
      error = explainError(e, $t);
    } finally {
      connectingId = null;
    }
  }

  async function handleDisconnect(conn: SshConnection) {
    const sessions = sshStore.sessionsForConnection(conn.id).filter(
      (s) => s.status === 'connected' || s.status === 'connecting'
    );
    for (const s of sessions) {
      try { await sshStore.disconnect(s.session_id); } catch {}
    }
  }

  async function confirmDelete() {
    try {
      await api.ssh.connectionDelete(deleteModal.id);
      sshStore.connections = sshStore.connections.filter((c) => c.id !== deleteModal.id);
    } catch (e) {
      error = explainError(e, $t);
    } finally {
      deleteModal = { open: false, id: '', name: '' };
    }
  }

  function onFormSaved(conn: SshConnection) {
    const exists = sshStore.connections.find((c) => c.id === conn.id);
    sshStore.connections = exists
      ? sshStore.connections.map((c) => c.id === conn.id ? conn : c)
      : [conn, ...sshStore.connections];
    panelConn = undefined;
  }

  function authLabel(auth: string) {
    if (auth === 'password') return $t('ssh_auth_badge_password');
    if (auth === 'key') return $t('ssh_auth_badge_key');
    if (auth === 'key_password') return $t('ssh_auth_badge_key_password');
    return auth;
  }

  function formatDateParts(iso: string | null): { date: string; time: string } {
    if (!iso) return { date: '—', time: '' };
    return { date: formatDate(iso, $locale), time: formatTime(iso, $locale) };
  }

  // The proxies belong to the browser module: names and the editor come through the catalog.
  function proxyName(id: string | null): string {
    if (!id) return '—';
    return directory.summary('proxy', id)?.name ?? id.slice(0, 8);
  }

  const editProxy = (id: string) => directory.get('proxy')?.edit?.(id);
</script>

<div class="page page--fill">
  <div class="page-header">
    <div class="page-title-group">
      <h1>{$t('terminal_title')}</h1>
      <p class="page-sub">{$t('terminal_sub', { count: String(sshStore.connections.length) })}</p>
    </div>
    {#if activeTab === 'connections'}
      <button class="btn btn-primary spacer" onclick={() => (panelConn = null)}>
        <Icon name="plus" size={15} />{$t('terminal_add')}
      </button>
    {/if}
  </div>

  <div class="tab-bar page-tabs">
    <button
      class="tab"
      class:active={activeTab === 'connections'}
      onclick={() => (activeTab = 'connections')}
    >
      <Icon name="terminal" size={12} /> {$t('ssh_keys_tab_connections')}
      {#if sshStore.connections.length > 0}
        <span class="tab-count">{sshStore.connections.length}</span>
      {/if}
    </button>
    <button
      class="tab"
      class:active={activeTab === 'keys'}
      onclick={() => (activeTab = 'keys')}
    >
      <Icon name="key" size={12} /> {$t('ssh_keys_tab_keys')}
    </button>
  </div>

  {#if activeTab === 'keys'}
    <SshKeysTab />
  {:else}

  {#if error}<div class="error-msg" style="margin-bottom:1rem">{error}</div>{/if}

  <!-- Toolbar -->
  <div class="filter-bar">
    <div class="search-field">
      <Icon name="search" size={13} />
      <input type="text" bind:value={search} placeholder={$t('terminal_search_placeholder')} />
      {#if search}
        <button class="search-clear" onclick={() => (search = '')}><Icon name="x" size={11} /></button>
      {/if}
    </div>

    <select bind:value={filterAuth} class="filter-select">
      <option value="all">{$t('terminal_filter_all_auth')}</option>
      <option value="password">{$t('ssh_auth_password')}</option>
      <option value="key">{$t('ssh_auth_key')}</option>
      <option value="key_password">{$t('ssh_auth_key_password')}</option>
    </select>

    <span class="count-badge">{$t('terminal_count', { n: String(filtered.length) })}</span>
  </div>

  {#if loading}
    <div class="empty-state">{$t('loading')}</div>
  {:else if sshStore.connections.length === 0}
    <div class="empty-state">
      <div class="empty-icon"><Icon name="terminal" size={40} strokeWidth={1.5} /></div>
      <p>{$t('terminal_empty')}</p>
      <button class="btn btn-primary" onclick={() => (panelConn = null)}>
        <Icon name="plus" size={14} />{$t('terminal_empty_add')}
      </button>
    </div>
  {:else if filtered.length === 0}
    <div class="empty-state">
      <Icon name="search" size={32} strokeWidth={1.5} />
      <p>{$t('terminal_not_found')}</p>
    </div>
  {:else}
    <div class="table-wrap">
      <table class="ssh-table">
        <thead>
          <tr>
            <th class="col-num">#</th>
            <th class="col-status"></th>
            <th class="col-name">{$t('terminal_col_name')}</th>
            <th class="col-host">{$t('terminal_col_host')}</th>
            <th class="col-user">{$t('terminal_col_user')}</th>
            <th class="col-auth">{$t('terminal_col_auth')}</th>
            <th class="col-proxy">{$t('terminal_col_proxy')}</th>
            <th class="col-last">{$t('terminal_col_last')}</th>
            <th class="col-actions">{$t('terminal_col_actions')}</th>
          </tr>
        </thead>
        <tbody>
          {#each pageItems as conn, i (conn.id)}
            {@const status = connStatus(conn.id)}
            {@const dp = formatDateParts(conn.last_connected_at)}
            <tr class="ssh-row" onclick={() => (panelConn = conn)}>
              <td class="col-num text-muted">{currentPage * PAGE_SIZE + i + 1}</td>
              <td class="col-status">
                <span class="status-dot status-dot-{status}" title={$t(`terminal_status_${status}`)}></span>
              </td>
              <td class="col-name">
                <span class="conn-name" title={conn.name}>{conn.name}</span>
              </td>
              <td class="col-host">
                <code title="{conn.host}:{conn.port}">{conn.host}:{conn.port}</code>
              </td>
              <td class="col-user">
                <span class="text-muted cell-text" title={conn.username}>{conn.username}</span>
              </td>
              <td class="col-auth">
                <div class="auth-tags">
                  <span class="auth-badge auth-{conn.auth_type}">{authLabel(conn.auth_type)}</span>
                  {#if conn.has_password && conn.auth_type !== 'password'}
                    <span class="auth-badge">{$t('ssh_auth_badge_password')}</span>
                  {/if}
                  {#if conn.requires_2fa}
                    <span class="tag-2fa">2FA</span>
                  {/if}
                </div>
              </td>
              <td class="col-proxy">
                {#if conn.proxy_id}
                  <button
                    class="proxy-chip"
                    title={proxyName(conn.proxy_id)}
                    onclick={(e) => { e.stopPropagation(); editProxy(conn.proxy_id!); }}
                  >{proxyName(conn.proxy_id)}</button>
                {:else}
                  <span class="text-muted">—</span>
                {/if}
              </td>
              <td class="col-last">
                <span class="date-cell">
                  <span>{dp.date}</span>
                  {#if dp.time}<span class="time-part">{dp.time}</span>{/if}
                </span>
              </td>
              <td class="col-actions">
                <div class="row-actions">
                  {#if status === 'connected' || status === 'connecting'}
                    <button
                      class="icon-btn danger-soft"
                      title={$t('terminal_btn_disconnect')}
                      onclick={(e) => { e.stopPropagation(); handleDisconnect(conn); }}
                    >
                      <Icon name="square" size={13} />
                    </button>
                  {:else}
                    <button
                      class="icon-btn success"
                      title={$t('terminal_btn_connect')}
                      disabled={connectingId === conn.id}
                      onclick={(e) => { e.stopPropagation(); handleConnect(conn); }}
                    >
                      <Icon name="terminal" size={13} />
                    </button>
                  {/if}
                  <button
                    class="icon-btn"
                    title={$t('terminal_btn_edit')}
                    onclick={(e) => { e.stopPropagation(); panelConn = conn; }}
                  >
                    <Icon name="pencil" size={13} />
                  </button>
                  <button
                    class="icon-btn danger-soft"
                    title={$t('terminal_btn_delete')}
                    onclick={(e) => { e.stopPropagation(); deleteModal = { open: true, id: conn.id, name: conn.name }; }}
                  >
                    <Icon name="trash-2" size={13} />
                  </button>
                </div>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    {#if totalPages > 1}
      <div class="pagination">
        <button class="page-btn" disabled={currentPage === 0} onclick={() => (page = currentPage - 1)}>
          <Icon name="chevron-left" size={14} />
        </button>
        {#each Array.from({ length: totalPages }, (_, i) => i) as p_}
          <button
            class="page-btn"
            class:active={p_ === currentPage}
            onclick={() => (page = p_)}
          >{p_ + 1}</button>
        {/each}
        <button class="page-btn" disabled={currentPage === totalPages - 1} onclick={() => (page = currentPage + 1)}>
          <Icon name="chevron-right" size={14} />
        </button>
      </div>
    {/if}
  {/if}

  {/if}
</div>

<Drawer
  open={panelConn !== undefined}
  width="440px"
  title={panelConn ? $t('ssh_form_edit') : $t('ssh_form_new')}
  onclose={() => (panelConn = undefined)}
>
  <SSHConnectionForm
    connection={panelConn}
    onSave={onFormSaved}
    onCancel={() => (panelConn = undefined)}
  />
</Drawer>

<Modal
  open={deleteModal.open}
  title={$t('terminal_btn_delete')}
  message={$t('terminal_confirm_delete', { name: deleteModal.name })}
  confirmLabel={$t('terminal_btn_delete')}
  cancelLabel={$t('ssh_btn_cancel')}
  variant="danger"
  onconfirm={confirmDelete}
  oncancel={() => (deleteModal = { open: false, id: '', name: '' })}
/>

<style>
  /* панель фильтров — единые примитивы .filter-bar/.search-field/.filter-select/.count-badge из base.css */

  .page-title-group { display: flex; flex-direction: column; gap: 6px; }

  /* Page-level tabs: connections | keys (base .tab-bar/.tab primitives) */
  .page-tabs { margin-bottom: var(--sp-4); padding: 0; }

  .table-wrap {
    flex: 1; min-height: 0; overflow-y: auto;
    border: 1px solid var(--border); border-radius: var(--radius-lg);
    background: var(--surface);
  }

  .ssh-table {
    width: 100%; border-collapse: collapse; font-size: var(--fs-sm);
    table-layout: fixed;
  }

  .ssh-table thead {
    position: sticky; top: 0; z-index: 1;
    background: var(--surface); border-bottom: 1px solid var(--border);
  }

  .ssh-table th {
    padding: var(--sp-3) var(--sp-4); text-align: left;
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--text-3);
    text-transform: uppercase; letter-spacing: 0.7px;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }

  .ssh-table td { padding: var(--sp-3) var(--sp-4); border-bottom: 1px solid var(--surface-2); vertical-align: middle; }
  .ssh-row:last-child td { border-bottom: none; }
  .ssh-row:hover td { background: var(--surface-row-hover); }
  .ssh-row { cursor: pointer; }

  /* The fixed layout takes the widths from the header row: the host (a mono
     host:port, the longest value) takes what the other columns leave, and
     every cell clips its text to its column (the full value is the cell's
     tooltip). */
  .col-num { width: 44px; color: var(--text-3); font-family: var(--font-mono); font-size: var(--fs-xs); }
  .col-status { width: 40px; text-align: center; }
  .col-name { width: 180px; }
  .col-user { width: 132px; }
  .col-auth { width: 130px; }
  .col-proxy { width: 120px; }
  .col-last { width: 128px; }
  /* three 32px buttons, two gaps and the cell's padding */
  .col-actions { width: calc(3 * 32px + 2 * var(--sp-1) + 2 * var(--sp-4)); }

  .cell-text { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .conn-name { font-weight: var(--fw-semibold); font-size: 0.9rem; color: var(--text); display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .auth-tags { display: flex; flex-wrap: wrap; gap: var(--sp-1); align-items: center; }

  code {
    font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-body);
    background: var(--surface-2); padding: 5px 10px; border-radius: 7px;
    display: inline-block; max-width: 100%; vertical-align: middle;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }

  .auth-badge {
    font-family: var(--font-mono);
    font-size: var(--fs-xs); font-weight: var(--fw-semibold);
    padding: 4px 12px; border-radius: var(--radius-sm);
    border: none; background: var(--accent-tint); color: var(--accent-text-2);
  }
  .auth-key { background: var(--accent-tint); color: var(--accent-text-2); }
  .auth-key_password { background: var(--warn-bg); color: var(--warn-text); }

  .tag-2fa { font-size: var(--fs-2xs); font-weight: 700; letter-spacing: 0.04em; padding: 0.15rem var(--sp-2); border-radius: 999px; background: var(--success-bg); border: 1px solid color-mix(in srgb, var(--success) 30%, var(--border)); color: var(--success-text); }
  .tag-proxy { font-size: var(--fs-2xs); font-weight: 700; letter-spacing: 0.04em; padding: 0.15rem var(--sp-2); border-radius: 999px; background: var(--surface-2); border: 1px solid var(--border); color: var(--text-3); }

  .status-dot {
    display: inline-block;
    width: 8px; height: 8px; border-radius: 50%;
  }
  .status-dot-connected { background: var(--success-text); box-shadow: 0 0 5px var(--success-text); }
  .status-dot-connecting {
    background: var(--warn-text);
    animation: dot-pulse 1s ease-in-out infinite;
  }
  .status-dot-idle { background: var(--border-2); }

  @keyframes dot-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
  }

  .date-cell { display: flex; flex-direction: column; gap: 0.05rem; }
  .date-cell span { color: var(--text-3); font-size: var(--fs-xs); }
  .time-part { color: var(--text-4, var(--text-3)); font-size: var(--fs-2xs); opacity: 0.75; }

  .proxy-chip {
    display: inline-block; max-width: 100%;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    background: var(--accent-bg); border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border));
    color: var(--accent); border-radius: 999px;
    font-size: var(--fs-2xs); font-weight: 700; letter-spacing: 0.03em;
    padding: 0.15rem var(--sp-2); cursor: pointer;
    transition: all 0.15s;
  }
  .proxy-chip:hover { background: color-mix(in srgb, var(--accent) 15%, var(--accent-bg)); }

  .text-muted { color: var(--text-3); font-size: var(--fs-xs); }
  .row-actions { display: flex; gap: var(--sp-1); flex-shrink: 0; }

  .pagination {
    display: flex; align-items: center; gap: var(--sp-1); justify-content: center;
    padding-top: var(--sp-1); flex-shrink: 0;
  }

  .page-btn {
    min-width: 30px; height: 30px; padding: 0 0.4rem;
    display: flex; align-items: center; justify-content: center;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: var(--radius-sm); color: var(--text-2);
    font-size: var(--fs-sm); cursor: pointer; transition: all 0.15s;
  }
  .page-btn:hover:not(:disabled) { background: var(--surface); border-color: var(--border-2); color: var(--text); }
  .page-btn.active { background: var(--accent-bg); border-color: var(--accent-border); color: var(--accent-text); }
  .page-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  /* uses global .empty-state */
  .empty-icon { opacity: 0.4; }

</style>

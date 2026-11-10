<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t, locale } from '$lib/core/i18n';
  import Notice from '$lib/core/ui/Notice.svelte';
  import Skeleton from '$lib/core/ui/Skeleton.svelte';
  import { formatDate, formatTime } from '$lib/core/utils';
  import type { SshConnection } from '$lib/ssh/types';
  import Icon from '$lib/core/Icon.svelte';
  import { pinnedHead } from '$lib/core/pinnedHead';
  import { ask, confirmTitle } from '$lib/core/ui/confirm.svelte';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import SSHConnectionForm from '$lib/ssh/components/SSHConnectionForm.svelte';
  import SshKeysTab from '$lib/ssh/components/keys/SshKeysTab.svelte';
  import { sshStore } from '$lib/ssh/store/ssh.svelte';
  import { directory } from '$lib/core/directory';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
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
  // The form in the drawer: its buttons are the drawer's footer.
  let connForm = $state<ReturnType<typeof SSHConnectionForm>>();
  let connSaving = $state(false);
  // The «Keys» tab's windows (import / generate): opened by the buttons of the page's head.
  let keyFormMode = $state<'import' | 'generate' | undefined>(undefined);
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

  async function confirmDelete(conn: SshConnection) {
    const label = $t('common_delete');
    if (!(await ask({ title: confirmTitle(label), message: $t('terminal_confirm_delete', { name: conn.name }), confirmLabel: label, variant: 'danger', icon: 'trash-2' }))) return;
    try {
      await api.ssh.connectionDelete(conn.id);
      sshStore.connections = sshStore.connections.filter((c) => c.id !== conn.id);
    } catch (e) {
      error = explainError(e, $t);
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
      <p class="page-sub">{$t('terminal_sub')}</p>
    </div>
    {#if activeTab === 'connections'}
      <button class="btn btn-primary spacer" onclick={() => (panelConn = null)}>
        <Icon name="plus" size={15} />{$t('terminal_new_connection')}
      </button>
    {:else}
      <button class="btn btn-ghost spacer" onclick={() => (keyFormMode = 'import')}>
        <Icon name="file-import" size={18} />{$t('ssh_keys_add')}
      </button>
      <button class="btn btn-primary" onclick={() => (keyFormMode = 'generate')}>
        <Icon name="plus" size={18} />{$t('ssh_keys_generate')}
      </button>
    {/if}
  </div>

  <div class="tab-bar page-tabs">
    <button
      class="tab"
      class:active={activeTab === 'connections'}
      onclick={() => (activeTab = 'connections')}
    >
      <Icon name="terminal" size={18} /> {$t('ssh_keys_tab_connections')}
    </button>
    <button
      class="tab"
      class:active={activeTab === 'keys'}
      onclick={() => (activeTab = 'keys')}
    >
      <Icon name="key" size={18} /> {$t('ssh_keys_tab_keys')}
    </button>
  </div>

  {#if activeTab === 'keys'}
    <SshKeysTab bind:formMode={keyFormMode} />
  {:else}

  {#if error}<div style="margin-bottom:1rem"><Notice tone="danger">{error}</Notice></div>{/if}

  <!-- Toolbar -->
  <div class="filter-bar">
    <div class="search-field">
      <Icon name="search" size={18} />
      <input type="text" bind:value={search} placeholder={$t('terminal_search_placeholder')} />
      {#if search}
        <button class="search-clear" onclick={() => (search = '')}><Icon name="x" size={18} /></button>
      {/if}
    </div>

    <div class="filter-select">
      <CustomSelect
        options={[
          { value: 'all', label: $t('terminal_filter_all_auth') },
          { value: 'password', label: $t('ssh_auth_password') },
          { value: 'key', label: $t('ssh_auth_key') },
          { value: 'key_password', label: $t('ssh_auth_key_password') },
        ]}
        bind:value={filterAuth}
      />
    </div>

    <span class="count-badge">{$t('terminal_count', { n: String(filtered.length) })}</span>
  </div>

  {#if loading}
    <div class="data-table table-scroll"><Skeleton rows={5} height={48} size={24} avatar="square" trailing /></div>
  {:else if sshStore.connections.length === 0}
    <div class="empty-state">
      <span class="empty-icon"><Icon name="terminal" size={22} /></span>
      <p>{$t('terminal_empty')}</p>
      <button class="btn btn-primary" onclick={() => (panelConn = null)}>
        <Icon name="plus" size={18} />{$t('terminal_empty_add')}
      </button>
    </div>
  {:else if filtered.length === 0}
    <div class="empty-state">
      <span class="empty-icon"><Icon name="search" size={22} /></span>
      <p>{$t('common_nothing_found')}</p>
      <span>{$t('common_nothing_found_hint', { q: search.trim() })}</span>
    </div>
  {:else}
    <div class="data-table table-scroll" use:pinnedHead>
      <table class="ssh-table pinned-head">
        <thead class="data-table-head">
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
            <tr class="data-table-row ssh-row" onclick={() => (panelConn = conn)}>
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
                  <span class="badge tag {conn.auth_type === 'key_password' ? 'badge-warn' : 'badge-accent'}">{authLabel(conn.auth_type)}</span>
                  {#if conn.has_password && conn.auth_type !== 'password'}
                    <span class="badge tag badge-accent">{$t('ssh_auth_badge_password')}</span>
                  {/if}
                  {#if conn.requires_2fa}
                    <span class="badge tag badge-ok">2FA</span>
                  {/if}
                </div>
              </td>
              <td class="col-proxy">
                {#if conn.proxy_id}
                  <button
                    class="badge tag proxy-chip"
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
                      <Icon name="plug-connected-x" size={18} />
                    </button>
                  {:else}
                    <button
                      class="icon-btn success"
                      title={$t('terminal_btn_connect')}
                      disabled={connectingId === conn.id}
                      onclick={(e) => { e.stopPropagation(); handleConnect(conn); }}
                    >
                      <Icon name="play" size={18} />
                    </button>
                  {/if}
                  <button
                    class="icon-btn"
                    title={$t('common_edit')}
                    onclick={(e) => { e.stopPropagation(); panelConn = conn; }}
                  >
                    <Icon name="pencil" size={18} />
                  </button>
                  <button
                    class="icon-btn danger-soft"
                    title={$t('common_delete')}
                    onclick={(e) => { e.stopPropagation(); void confirmDelete(conn); }}
                  >
                    <Icon name="trash-2" size={18} />
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
          <Icon name="chevron-left" size={18} />
        </button>
        {#each Array.from({ length: totalPages }, (_, i) => i) as p_}
          <button
            class="page-btn"
            class:active={p_ === currentPage}
            onclick={() => (page = p_)}
          >{p_ + 1}</button>
        {/each}
        <button class="page-btn" disabled={currentPage === totalPages - 1} onclick={() => (page = currentPage + 1)}>
          <Icon name="chevron-right" size={18} />
        </button>
      </div>
    {/if}
  {/if}

  {/if}
</div>

<Drawer
  open={panelConn !== undefined}
  width="var(--drawer-w-md)"
  title={panelConn ? $t('ssh_form_edit') : $t('ssh_form_new')}
  onclose={() => (panelConn = undefined)}
>
  <SSHConnectionForm
    bind:this={connForm}
    bind:saving={connSaving}
    connection={panelConn}
    onSave={onFormSaved}
  />
  {#snippet footer()}
    <button class="btn btn-ghost" onclick={() => (panelConn = undefined)}>{$t('common_cancel')}</button>
    <button class="btn btn-primary" onclick={() => connForm?.submit()} disabled={connSaving}>
      {#if connSaving}<Icon name="loader" size={18} />{/if}
      {panelConn ? $t('common_save') : $t('common_create')}
    </button>
  {/snippet}
</Drawer>

<style>
  /* панель фильтров — единые примитивы .filter-bar/.search-field/.filter-select/.count-badge из base.css */

  .page-title-group { display: flex; flex-direction: column; gap: 6px; }

  /* Page-level tabs: connections | keys (base .tab-bar/.tab primitives) */
  .page-tabs { margin-bottom: var(--sp-4); padding: 0; }

  /* Table: the common glass .data-table, as high as its rows, scrolled inside when the page is short. */
  .table-scroll { flex: 0 1 auto; min-height: 0; overflow-y: auto; }

  .ssh-table { width: 100%; font-size: var(--fs-sm); }

  .ssh-table th {
    padding: 0 var(--sp-4); text-align: left;
    font: inherit; white-space: nowrap;
  }

  .ssh-table td { padding: var(--sp-3) var(--sp-4); vertical-align: middle; }
  .ssh-row { cursor: pointer; }

  .col-num { width: 44px; color: var(--text-3); font-family: var(--font-mono); font-size: var(--fs-xs); }
  .col-status { width: 40px; text-align: center; }
  /* the buttons stay on one line */
  .col-actions { width: 1%; }

  .cell-text { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .conn-name { font-weight: var(--fw-semibold); font-size: var(--fs-body); color: var(--text); display: block; overflow-wrap: anywhere; }

  .auth-tags { display: flex; flex-wrap: wrap; gap: var(--sp-1); align-items: center; }

  code {
    font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-body);
    background: var(--surface-2); padding: 5px 10px; border-radius: var(--radius-sm);
    display: inline-block; max-width: 100%; vertical-align: middle; overflow-wrap: anywhere;
  }

  /* A word in a chip (the common .badge): 24 high, the interface's font. */
  .tag { height: 24px; padding-block: 0; box-sizing: border-box; }

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
  .time-part { color: var(--text-4, var(--text-3)); font-size: var(--fs-xs); opacity: 0.75; }

  /* The proxy: a neutral chip of an entity that opens its editor. */
  .proxy-chip { display: inline-block; max-width: 100%; line-height: 22px; overflow: hidden; text-overflow: ellipsis; cursor: pointer; font-family: inherit; }
  .proxy-chip:hover { border-color: var(--border-2); color: var(--text); }

  .text-muted { color: var(--text-3); font-size: var(--fs-xs); }
  .row-actions { display: flex; gap: var(--sp-1); flex-shrink: 0; }

  /* A narrow window: a row folds — the name on top, the host under it, the state and the buttons at the right; the
     user, the authentication, the proxy and the date a line below. No column is lost. */
  @media (max-width: 900px) {
    .ssh-table thead { display: none; }
    .ssh-table tbody { display: block; }
    .ssh-row {
      display: grid; align-items: center; column-gap: var(--sp-3); row-gap: var(--sp-1); padding: var(--sp-3) var(--sp-4);
      /* The first column is the number's (fixed, up to four digits): the name spans it, the user starts after it. */
      grid-template-columns: 2.5rem auto auto auto minmax(0, 1fr) auto auto;
      grid-template-areas: 'name name name name name st act' 'host host host host host st act' 'num user auth proxy last last last';
    }
    .ssh-table td { display: block; width: auto; border: 0; padding: 0; min-width: 0; }
    .ssh-row .col-num { grid-area: num; }
    .ssh-row .col-status { grid-area: st; }
    .ssh-row .col-name { grid-area: name; }
    .ssh-row .col-host { grid-area: host; }
    .ssh-row .col-user { grid-area: user; }
    .ssh-row .col-auth { grid-area: auth; }
    .ssh-row .col-proxy { grid-area: proxy; }
    .ssh-row .col-last { grid-area: last; }
    .ssh-row .col-actions { grid-area: act; }
    .date-cell { flex-direction: row; gap: var(--sp-1); }
  }

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
  .page-btn:disabled { opacity: 0.45; cursor: not-allowed; }


</style>

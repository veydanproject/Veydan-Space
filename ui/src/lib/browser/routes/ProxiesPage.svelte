<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/browser/api';
  import { t } from '$lib/core/i18n';
  import type { Proxy, ProxyCheckResult } from '$lib/browser/types';
  import Icon from '$lib/core/Icon.svelte';
  import Modal from '$lib/core/ui/Modal.svelte';
  import ProxyPanel from '$lib/browser/components/ProxyPanel.svelte';
  import BulkImportProxyModal from '$lib/browser/components/BulkImportProxyModal.svelte';
  import { proxiesStore } from '$lib/browser/store/proxies.svelte';
  import { formatError } from '$lib/core/utils';
  import { explainError } from '$lib/browser/tor-error';
  import ProxyAddress from '$lib/browser/components/ProxyAddress.svelte';

  const PAGE_SIZE = 20;

  let loading = $state(false);
  let error = $state('');

  let search = $state('');
  let filterType = $state('all');
  let filterStatus = $state('all');
  let page = $state(0);

  let panelProxy = $state<Proxy | null | undefined>(undefined);
  let importOpen = $state(false);
  let checkResults = $state<Record<string, ProxyCheckResult & { checking?: boolean; err?: string }>>({});
  let deleteModal = $state({ open: false, id: '', name: '', usage: '' });

  /** Open the confirmation, then fill in what the delete would detach. */
  async function askDelete(proxy: Proxy) {
    deleteModal = { open: true, id: proxy.id, name: proxy.name, usage: '' };
    const usage = await proxiesStore.deleteWarning(proxy.id);
    // The user may have cancelled or picked another row while this was in flight.
    if (deleteModal.open && deleteModal.id === proxy.id) deleteModal.usage = usage;
  }

  onMount(async () => {
    loading = true;
    try { await proxiesStore.ensureLoaded(); }
    catch (e) { error = formatError(e); }
    finally { loading = false; }
  });

  let filtered = $derived(proxiesStore.list.filter((p) => {
    if (filterType !== 'all' && p.proxy_type !== filterType) return false;
    if (filterStatus !== 'all' && p.status !== filterStatus) return false;
    if (search.trim()) {
      const q = search.toLowerCase();
      return (
        p.name.toLowerCase().includes(q) ||
        p.host.toLowerCase().includes(q) ||
        (p.proxy_type === 'tor' && 'tor'.includes(q)) ||
        (p.country ?? '').toLowerCase().includes(q) ||
        (p.last_ip ?? '').includes(q)
      );
    }
    return true;
  }));

  let totalPages = $derived(Math.max(1, Math.ceil(filtered.length / PAGE_SIZE)));
  let currentPage = $derived(Math.min(page, totalPages - 1));
  let pageItems = $derived(filtered.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE));

  $effect(() => {
    // сброс страницы при изменении фильтров
    search; filterType; filterStatus;
    page = 0;
  });

  let fingerprintPrompt = $state<{ id: string; fingerprint: string; ip: string; country: string | null; city: string | null } | null>(null);

  async function checkProxy(id: string) {
    checkResults = { ...checkResults, [id]: { ip: '', country: null, city: null, ok: false, checking: true } };
    try {
      const result = await api.proxies.check(id);
      checkResults = { ...checkResults, [id]: { ...result, checking: false } };

      if (result.ssh_fingerprint_is_new && result.ssh_fingerprint) {
        fingerprintPrompt = { id, fingerprint: result.ssh_fingerprint, ip: result.ip, country: result.country, city: result.city };
        return;
      }

      proxiesStore.list = proxiesStore.list.map((p) => {
        if (p.id !== id) return p;
        return {
          ...p,
          status: 'active',
          last_ip: result.ip,
          country: p.country || result.country,
          city: p.city || result.city,
        };
      });
    } catch (e) {
      checkResults = { ...checkResults, [id]: { ip: '', country: null, city: null, ok: false, checking: false, err: explainError(e, $t) } };
      proxiesStore.list = proxiesStore.list.map((p) => p.id === id ? { ...p, status: 'failed' } : p);
    }
  }

  async function trustFingerprint() {
    if (!fingerprintPrompt) return;
    const { id, fingerprint, ip, country, city } = fingerprintPrompt;
    try {
      await api.proxies.trustFingerprint(id, fingerprint, ip, country, city);
      proxiesStore.list = proxiesStore.list.map((p) => p.id === id
        ? { ...p, status: 'active', last_ip: ip, server_fingerprint: fingerprint, country: p.country || country, city: p.city || city }
        : p
      );
    } catch (e) { error = formatError(e); }
    finally { fingerprintPrompt = null; }
  }

  async function confirmDelete() {
    try {
      await api.proxies.delete(deleteModal.id);
      proxiesStore.list = proxiesStore.list.filter((p) => p.id !== deleteModal.id);
    } catch (e) { error = formatError(e); }
    finally { deleteModal = { open: false, id: '', name: '', usage: '' }; }
  }

  function onPanelSaved(proxy: Proxy) {
    const exists = proxiesStore.list.find((p) => p.id === proxy.id);
    proxiesStore.list = exists
      ? proxiesStore.list.map((p) => p.id === proxy.id ? proxy : p)
      : [proxy, ...proxiesStore.list];
    panelProxy = undefined;
  }
</script>

<div class="page page--fill">
  <div class="page-header">
    <div class="page-title-group">
      <h1>{$t('proxies_title')}</h1>
      <p class="page-sub">{$t('proxies_sub', { count: String(proxiesStore.list.length) })}</p>
    </div>
    <button class="btn btn-ghost spacer" onclick={() => (importOpen = true)}>
      <Icon name="upload" size={15} />{$t('proxies_import_btn')}
    </button>
    <button class="btn btn-primary" onclick={() => (panelProxy = null)}>
      <Icon name="plus" size={15} />{$t('proxies_add')}
    </button>
  </div>

  {#if error}<div class="error-msg" style="margin-bottom:1rem">{error}</div>{/if}

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

  <!-- Toolbar -->
  <div class="filter-bar">
    <div class="search-field">
      <Icon name="search" size={13} />
      <input type="text" bind:value={search} placeholder={$t('proxies_search_placeholder')} />
      {#if search}
        <button class="search-clear" onclick={() => (search = '')}><Icon name="x" size={11} /></button>
      {/if}
    </div>

    <select bind:value={filterType} class="filter-select">
      <option value="all">{$t('proxies_filter_all_types')}</option>
      <option value="http">HTTP</option>
      <option value="https">HTTPS</option>
      <option value="socks5">SOCKS5</option>
      <option value="ssh">SSH</option>
      <option value="tor">{'Tor'}</option>
    </select>
    <select bind:value={filterStatus} class="filter-select">
      <option value="all">{$t('proxies_filter_all_statuses')}</option>
      <option value="active">{$t('proxies_filter_active')}</option>
      <option value="failed">{$t('proxies_filter_failed')}</option>
      <option value="unknown">{$t('proxies_filter_unknown')}</option>
    </select>

    <span class="count-badge">{$t('proxies_count', { n: String(filtered.length) })}</span>
  </div>

  {#if loading}
    <div class="empty-state">{$t('loading')}</div>
  {:else if proxiesStore.list.length === 0}
    <div class="empty-state">
      <div class="empty-icon"><Icon name="network" size={40} strokeWidth={1.5} /></div>
      <p>{$t('proxies_empty')}</p>
      <button class="btn btn-primary" onclick={() => (panelProxy = null)}>
        <Icon name="plus" size={14} />{$t('proxies_empty_add')}
      </button>
    </div>
  {:else if filtered.length === 0}
    <div class="empty-state">
      <Icon name="search" size={32} strokeWidth={1.5} />
      <p>{$t('proxies_not_found')}</p>
    </div>
  {:else}
    <div class="table-wrap">
      <table class="proxy-table">
        <thead>
          <tr>
            <th class="col-num">#</th>
            <th>{$t('proxy_col_name')}</th>
            <th class="col-type">{$t('proxy_col_type')}</th>
            <th>{$t('proxy_col_host')}</th>
            <th class="col-status">{$t('proxy_col_status')}</th>
            <th class="col-geo">{$t('proxy_col_country')}</th>
            <th class="col-actions">{$t('proxy_col_actions')}</th>
          </tr>
        </thead>
        <tbody>
            {#each pageItems as proxy, i (proxy.id)}
            {@const result = checkResults[proxy.id]}
            <tr class="proxy-row" onclick={() => (panelProxy = proxy)}>
              <td class="col-num text-muted">{currentPage * PAGE_SIZE + i + 1}</td>
              <td class="col-name">
                <span class="proxy-name">{proxy.name}</span>
              </td>
              <td class="col-type">
                <span class="type-badge type-{proxy.proxy_type}">{proxy.proxy_type}</span>
              </td>
              <td class="col-host">
                <ProxyAddress {proxy} {result} />
              </td>
              <td class="col-status">
                <span class="status-badge status-{proxy.status}">{$t(`proxies_filter_${proxy.status}`)}</span>
              </td>
              <td class="col-geo">
                {#if proxy.country}
                  <span class="geo-chip">{proxy.country}{proxy.city ? ` / ${proxy.city}` : ''}</span>
                {:else}
                  <span class="text-muted">—</span>
                {/if}
              </td>
              <td class="col-actions">
                  <div class="row-actions">
                  <button
                    class="icon-btn"
                    title={$t('proxy_btn_check')}
                    disabled={result?.checking}
                    onclick={(e) => { e.stopPropagation(); checkProxy(proxy.id); }}
                  >
                    <Icon name="refresh-cw" size={13} />
                  </button>
                  <button
                    class="icon-btn"
                    title={$t('proxy_btn_edit')}
                    onclick={(e) => { e.stopPropagation(); panelProxy = proxy; }}
                  >
                    <Icon name="pencil" size={13} />
                  </button>
                  <button
                    class="icon-btn danger-soft"
                    title={$t('proxy_btn_delete')}
                    onclick={(e) => { e.stopPropagation(); askDelete(proxy); }}
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
</div>

<BulkImportProxyModal open={importOpen} onclose={() => (importOpen = false)} />

{#if panelProxy !== undefined}
  <ProxyPanel
    proxy={panelProxy}
    onclose={() => (panelProxy = undefined)}
    onsaved={onPanelSaved}
  />
{/if}

<Modal
  open={deleteModal.open}
  title={$t('proxy_btn_delete')}
  message={$t('proxy_confirm_delete', { name: deleteModal.name }) + deleteModal.usage}
  confirmLabel={$t('proxy_btn_delete')}
  cancelLabel={$t('proxy_btn_cancel')}
  variant="danger"
  onconfirm={confirmDelete}
  oncancel={() => (deleteModal = { open: false, id: '', name: '', usage: '' })}
/>

<style>
  /* панель фильтров — единые примитивы .filter-bar/.search-field/.filter-select/.count-badge из base.css */
  /* The placeholder names four fields: the width of the SSH keys search, so it is read whole. */
  .filter-bar .search-field { width: 300px; }

  .page-title-group { display: flex; flex-direction: column; gap: 6px; }

  /* Table */
  .table-wrap {
    flex: 1; min-height: 0; overflow-y: auto;
    border: 1px solid var(--border); border-radius: var(--radius-lg);
    background: var(--surface);
  }

  .proxy-table {
    width: 100%; border-collapse: collapse; font-size: var(--fs-sm);
  }

  .proxy-table thead {
    position: sticky; top: 0; z-index: 1;
    background: var(--surface); border-bottom: 1px solid var(--border);
  }

  .proxy-table th {
    padding: var(--sp-3) var(--sp-4); text-align: left;
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--text-3);
    text-transform: uppercase; letter-spacing: 0.7px;
    white-space: nowrap;
  }

  .proxy-table td { padding: var(--sp-3) var(--sp-4); border-bottom: 1px solid var(--surface-2); vertical-align: middle; }
  .proxy-row:last-child td { border-bottom: none; }
  .proxy-row:hover td { background: var(--surface-row-hover); }
  .proxy-row { cursor: pointer; }

  .col-num { width: 44px; color: var(--text-3); font-family: var(--font-mono); font-size: var(--fs-xs); }
  .col-type { width: 100px; }
  .col-status { width: 120px; }
  .col-geo { width: 120px; }
  .col-actions { width: 120px; }

  .proxy-name { font-weight: var(--fw-semibold); font-size: 0.9rem; color: var(--text); display: block; }

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

  .geo-chip { font-size: var(--fs-xs); color: var(--text-2); }
  .text-muted { color: var(--text-3); font-size: var(--fs-xs); }

  .row-actions { display: flex; gap: var(--sp-1); }

  /* Pagination */
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

  /* Empty (uses global .empty-state) */
  .empty-icon { opacity: 0.4; }

  .fingerprint-banner {
    display: flex; align-items: flex-start; gap: var(--sp-3);
    background: color-mix(in srgb, var(--color-warning) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-warning) 40%, transparent);
    border-radius: var(--radius); padding: 0.875rem var(--sp-4); margin-bottom: var(--sp-4);
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

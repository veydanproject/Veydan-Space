<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/browser/api';
  import { t } from '$lib/core/i18n';
  import type { Proxy, ProxyCheckResult } from '$lib/browser/types';
  import Icon from '$lib/core/Icon.svelte';
  import { pinnedHead } from '$lib/core/pinnedHead';
  import Notice from '$lib/core/ui/Notice.svelte';
  import Skeleton from '$lib/core/ui/Skeleton.svelte';
  import StatePill, { type PillTone } from '$lib/core/ui/StatePill.svelte';
  import { ask, confirmTitle } from '$lib/core/ui/confirm.svelte';
  import ProxyPanel from '$lib/browser/components/ProxyPanel.svelte';
  import BulkImportProxyModal from '$lib/browser/components/BulkImportProxyModal.svelte';
  import { proxiesStore } from '$lib/browser/store/proxies.svelte';
  import { formatError } from '$lib/core/utils';
  import { explainError } from '$lib/browser/tor-error';
  import ProxyAddress from '$lib/browser/components/ProxyAddress.svelte';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';

  const PAGE_SIZE = 20;

  /** The colour of a type's chip (`.tinted`): http/https blue, socks5 purple, ssh teal, tor pink. */
  const TYPE_CHIP: Record<string, string> = {
    http: 'var(--cat-blue)', https: 'var(--cat-blue)', socks5: 'var(--cat-purple)', ssh: 'var(--cat-teal)', tor: 'var(--cat-pink)',
  };
  const STATUS_TONE: Record<string, PillTone> = { active: 'ok', failed: 'bad', unknown: 'neutral' };

  let loading = $state(false);
  let error = $state('');

  let search = $state('');
  let filterType = $state('all');
  let filterStatus = $state('all');
  let page = $state(0);

  let panelProxy = $state<Proxy | null | undefined>(undefined);
  let importOpen = $state(false);
  let checkResults = $state<Record<string, ProxyCheckResult & { checking?: boolean; err?: string }>>({});
  /** Ask, saying what the delete would detach, then delete. */
  async function askDelete(proxy: Proxy) {
    const usage = await proxiesStore.deleteWarning(proxy.id);
    const label = $t('common_delete');
    if (!(await ask({ title: confirmTitle(label), message: $t('proxy_confirm_delete', { name: proxy.name }) + usage, confirmLabel: label, variant: 'danger', icon: 'trash-2' }))) return;
    await confirmDelete(proxy.id);
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

  async function confirmDelete(id: string) {
    try {
      await api.proxies.delete(id);
      proxiesStore.list = proxiesStore.list.filter((p) => p.id !== id);
    } catch (e) { error = formatError(e); }
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
      <p class="page-sub">{$t('proxies_sub')}</p>
    </div>
    <button class="btn btn-ghost spacer" onclick={() => (importOpen = true)}>
      <Icon name="file-import" size={18} />{$t('proxies_import_btn')}
    </button>
    <button class="btn btn-primary" onclick={() => (panelProxy = null)}>
      <Icon name="plus" size={15} />{$t('proxies_new')}
    </button>
  </div>

  {#if error}<div class="plate"><Notice tone="danger">{error}</Notice></div>{/if}

  {#if fingerprintPrompt}
    <!-- A new server: the one plate of a server's key (SSHTerminal, FilePanel), «Trust» a plain secondary button. -->
    <div class="plate">
      <Notice tone="warn" role="alert">
        <b>{$t('ssh_fingerprint_new_title')}</b>
        <code class="fingerprint-fp">{fingerprintPrompt.fingerprint}</code>
        {$t('ssh_fingerprint_new_hint')}
        {#snippet action()}
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => fingerprintPrompt = null}>{$t('common_cancel')}</button>
          <button class="btn btn-ghost btn-sm" type="button" onclick={trustFingerprint}>{$t('ssh_fingerprint_trust')}</button>
        {/snippet}
      </Notice>
    </div>
  {/if}

  <!-- Toolbar -->
  <div class="filter-bar">
    <div class="search-field">
      <Icon name="search" size={18} />
      <input type="text" bind:value={search} placeholder={$t('proxies_search_placeholder')} />
      {#if search}
        <button class="search-clear" onclick={() => (search = '')}><Icon name="x" size={18} /></button>
      {/if}
    </div>

    <div class="filter-select">
      <CustomSelect
        options={[
          { value: 'all', label: $t('proxies_filter_all_types') },
          { value: 'http', label: 'HTTP' },
          { value: 'https', label: 'HTTPS' },
          { value: 'socks5', label: 'SOCKS5' },
          { value: 'ssh', label: 'SSH' },
          { value: 'tor', label: 'Tor' },
        ]}
        bind:value={filterType}
      />
    </div>
    <div class="filter-select">
      <CustomSelect
        options={[
          { value: 'all', label: $t('proxies_filter_all_statuses') },
          { value: 'active', label: $t('proxies_filter_active') },
          { value: 'failed', label: $t('proxies_filter_failed') },
          { value: 'unknown', label: $t('proxies_filter_unknown') },
        ]}
        bind:value={filterStatus}
      />
    </div>

    <span class="count-badge">{$t('proxies_count', { n: String(filtered.length) })}</span>
  </div>

  {#if loading}
    <div class="data-table table-scroll"><Skeleton rows={5} height={48} avatar="none" trailing /></div>
  {:else if proxiesStore.list.length === 0}
    <div class="empty-state">
      <span class="empty-icon"><Icon name="network" size={22} /></span>
      <p>{$t('proxies_empty')}</p>
      <button class="btn btn-primary" onclick={() => (panelProxy = null)}>
        <Icon name="plus" size={18} />{$t('proxies_empty_add')}
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
      <table class="proxy-table pinned-head">
        <thead class="data-table-head">
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
            <tr class="data-table-row proxy-row" onclick={() => (panelProxy = proxy)}>
              <td class="col-num text-muted">{currentPage * PAGE_SIZE + i + 1}</td>
              <td class="col-name">
                <span class="proxy-name">{proxy.name}</span>
              </td>
              <td class="col-type">
                <span class="type-chip tinted" style:--chip={TYPE_CHIP[proxy.proxy_type]}>{proxy.proxy_type}</span>
              </td>
              <td class="col-host">
                <ProxyAddress {proxy} {result} />
              </td>
              <td class="col-status">
                <StatePill text={$t(`proxies_filter_${proxy.status}`)} tone={STATUS_TONE[proxy.status] ?? 'neutral'} />
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
                    <Icon name="refresh-cw" size={18} />
                  </button>
                  <button
                    class="icon-btn"
                    title={$t('common_edit')}
                    onclick={(e) => { e.stopPropagation(); panelProxy = proxy; }}
                  >
                    <Icon name="pencil" size={18} />
                  </button>
                  <button
                    class="icon-btn danger-soft"
                    title={$t('common_delete')}
                    onclick={(e) => { e.stopPropagation(); askDelete(proxy); }}
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
</div>

<BulkImportProxyModal open={importOpen} onclose={() => (importOpen = false)} />

{#if panelProxy !== undefined}
  <ProxyPanel
    proxy={panelProxy}
    onclose={() => (panelProxy = undefined)}
    onsaved={onPanelSaved}
  />
{/if}

<style>
  /* панель фильтров — единые примитивы .filter-bar/.search-field/.filter-select/.count-badge из base.css */
  /* The placeholder names four fields: the width of the SSH keys search, so it is read whole. */
  .filter-bar .search-field { width: 300px; }

  .page-title-group { display: flex; flex-direction: column; gap: 6px; }

  /* Table: the common glass .data-table, as high as its rows, scrolled inside when the page is short. */
  .table-scroll { flex: 0 1 auto; min-height: 0; overflow-y: auto; }

  .proxy-table { width: 100%; font-size: var(--fs-sm); }

  .proxy-table th {
    padding: 0 var(--sp-4); text-align: left;
    font: inherit; white-space: nowrap;
  }

  .proxy-table td { padding: var(--sp-3) var(--sp-4); vertical-align: middle; }
  .proxy-row { cursor: pointer; }

  .col-num { width: 44px; color: var(--text-3); font-family: var(--font-mono); font-size: var(--fs-xs); }
  .col-type { width: 100px; }
  .col-status { width: 120px; }
  .col-geo { width: 120px; }
  .col-actions { width: 120px; }

  .proxy-name { font-weight: var(--fw-semibold); font-size: var(--fs-body); color: var(--text); display: block; overflow-wrap: anywhere; }

  /* The type: a chip in the interface's font, its colour by `--chip` (`.tinted`). */
  .type-chip {
    display: inline-flex; align-items: center; height: 24px; padding: 0 var(--sp-3); box-sizing: border-box;
    border: 1px solid; border-radius: var(--radius-pill);
    font-size: var(--fs-xs); font-weight: var(--fw-semibold); white-space: nowrap;
  }

  /* A narrow window: a row folds — the name and the type on top, the host under them, the state and the buttons at
     the right, the number and the country a line below. No column is lost. */
  @media (max-width: 900px) {
    .proxy-table thead { display: none; }
    .proxy-table tbody { display: block; }
    .proxy-row {
      display: grid; align-items: center; column-gap: var(--sp-3); row-gap: var(--sp-1); padding: var(--sp-3) var(--sp-4);
      /* The first column is the number's (fixed, up to four digits): the name spans it, the country starts after it. */
      grid-template-columns: 2.5rem minmax(0, max-content) auto 1fr auto auto;
      grid-template-areas: 'name name type . st act' 'host host host host st act' 'num geo geo geo geo geo';
    }
    .proxy-table td { display: block; width: auto; border: 0; padding: 0; min-width: 0; }
    .proxy-row .col-num { grid-area: num; }
    .proxy-row .col-name { grid-area: name; }
    .proxy-row .col-type { grid-area: type; }
    .proxy-row .col-host { grid-area: host; }
    .proxy-row .col-status { grid-area: st; }
    .proxy-row .col-geo { grid-area: geo; }
    .proxy-row .col-actions { grid-area: act; }
  }

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
  .page-btn:disabled { opacity: 0.45; cursor: not-allowed; }


  .plate { margin-bottom: var(--sp-4); }
  .fingerprint-fp { display: block; margin: var(--sp-1) 0; font-family: var(--font-mono); font-size: var(--fs-xs); word-break: break-all; }
</style>

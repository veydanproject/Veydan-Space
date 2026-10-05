<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { ask } from '$lib/core/ui/confirm.svelte';
  import type { Profile, Proxy, WorkspaceColumn } from '$lib/browser/types';
  import { t, locale } from '$lib/core/i18n';
  import { api, leaseHolder } from '$lib/browser/api';
  import ProfileSyncBadge from '$lib/browser/components/ProfileSyncBadge.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { formatDateTime } from '$lib/core/utils';
  import { proxyChip, proxyTypeName } from '$lib/browser/proxy-label';

  interface Props {
    profiles: Profile[];
    proxies: Proxy[];
    columns: WorkspaceColumn[];
    runningProfiles: Set<string>;
    onSelect: (profile: Profile) => void;
    onEdit: (profile: Profile) => void;
    onRefresh: () => void;
  }

  let { profiles, proxies, columns, runningProfiles, onSelect, onEdit, onRefresh }: Props = $props();

  let search = $state('');
  let filterTag = $state('');
  let filterStatus = $state('');
  let loadingId = $state<string | null>(null);

  const proxyMap = $derived(new Map(proxies.map((p) => [p.id, p])));

  const allTags = $derived(() => {
    const set = new Set<string>();
    for (const p of profiles) {
      for (const tg of (p.tags ?? [])) set.add(tg);
    }
    return [...set].sort();
  });

  const filtered = $derived(() => {
    let res = profiles;
    if (search) {
      const q = search.toLowerCase();
      res = res.filter((p) => p.name.toLowerCase().includes(q));
    }
    if (filterTag) {
      res = res.filter((p) => (p.tags ?? []).includes(filterTag));
    }
    if (filterStatus === 'running') res = res.filter((p) => runningProfiles.has(p.id));
    if (filterStatus === 'stopped') res = res.filter((p) => !runningProfiles.has(p.id));
    return res;
  });

  function getColumnForProfile(profile: Profile): string {
    const tags: string[] = profile.tags ?? [];
    for (const col of columns) {
      if (tags.includes(col.tag_name)) return col.name;
    }
    return '—';
  }

  const tagColorMap = $derived(
    new Map(columns.map((col) => [col.tag_name, col.color]))
  );

  function getOsIcon(preset: string): string {
    if (preset.startsWith('win')) return '🪟';
    if (preset === 'macos') return '🍎';
    if (preset === 'linux') return '🐧';
    return '💻';
  }

  function getOsLabel(preset: string): string {
    const map: Record<string, string> = {
      win10: 'Win 10', win11: 'Win 11', macos: 'macOS', linux: 'Linux',
    };
    return map[preset] ?? preset;
  }

  async function launch(e: MouseEvent, profile: Profile) {
    e.stopPropagation();
    loadingId = profile.id;
    try { await api.profiles.launch(profile.id); onRefresh(); }
    catch (err) {
      // Leased by another device: let the user override.
      const holder = leaseHolder(err);
      if (holder !== null && (await ask({ title: $t('profile_sync_in_use_confirm', { device: holder }), variant: 'primary' }))) {
        try { await api.profiles.launch(profile.id, true); onRefresh(); } catch {}
      }
    }
    finally { loadingId = null; }
  }

  async function stop(e: MouseEvent, profile: Profile) {
    e.stopPropagation();
    loadingId = profile.id;
    try { await api.profiles.stop(profile.id); onRefresh(); }
    catch {}
    finally { loadingId = null; }
  }

  async function clone(e: MouseEvent, profile: Profile) {
    e.stopPropagation();
    loadingId = profile.id;
    try { await api.profiles.clone(profile.id); onRefresh(); }
    catch (err) { console.error('clone failed', err); }
    finally { loadingId = null; }
  }

  function clearFilters() { search = ''; filterTag = ''; filterStatus = ''; }
  const hasFilter = $derived(!!search || !!filterTag || !!filterStatus);
</script>

<div class="table-view">
  <!-- Toolbar -->
  <div class="filter-bar">
    <div class="search-field">
      <Icon name="search" size={13} />
      <input type="text" placeholder={$t('kanban_filter_search')} bind:value={search} />
      {#if search}
        <button class="search-clear" onclick={() => (search = '')}><Icon name="x" size={11} /></button>
      {/if}
    </div>
    <select class="filter-select" bind:value={filterStatus}>
      <option value="">{$t('kanban_filter_runtime')}</option>
      <option value="running">{$t('kanban_filter_running')}</option>
      <option value="stopped">{$t('kanban_filter_stopped')}</option>
    </select>
    <select class="filter-select" bind:value={filterTag}>
      <option value="">{$t('filter_all_tags')}</option>
      {#each allTags() as tag}
        <option value={tag}>{tag}</option>
      {/each}
    </select>
    {#if hasFilter}
      <button class="filter-clear" onclick={clearFilters} title={$t('kanban_clear_filters')} aria-label={$t('kanban_clear_filters')}>
        <Icon name="x" size={12} />
      </button>
    {/if}
    <span class="count-badge">{filtered().length} / {profiles.length}</span>
  </div>

  <!-- Table -->
  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          <th class="th-num">#</th>
          <th>{$t('table_col_name')}</th>
          <th>{$t('table_col_os')}</th>
          <th>{$t('table_col_proxy')}</th>
          <th>{$t('table_col_tags')}</th>
          <th>{$t('table_col_last')}</th>
          <th class="th-actions">{$t('proxy_col_actions')}</th>
        </tr>
      </thead>
      <tbody>
        {#each filtered() as profile, i (profile.id)}
          {@const isRunning = runningProfiles.has(profile.id)}
          {@const isLoading = loadingId === profile.id}
          {@const proxy = proxyMap.get(profile.proxy_id ?? '')}
          <tr class:running={isRunning} onclick={() => onSelect(profile)}>

            <!-- # -->
            <td class="td-num">
              <span class="row-num">{i + 1}</span>
              <span class="status-dot" class:active={isRunning}></span>
            </td>

            <!-- Name -->
            <td class="td-name">
              <span class="profile-name">{profile.name}</span>
              {#if isRunning}
                <span class="running-pill">● live</span>
              {/if}
              <ProfileSyncBadge profileId={profile.id} compact />
            </td>

            <!-- OS / Browser -->
            <td class="td-os">
              <span class="os-chip">
                <span class="os-icon">{getOsIcon(profile.fingerprint_preset)}</span>
                <span class="os-label">{getOsLabel(profile.fingerprint_preset)}</span>
                <span class="sep">·</span>
                <span class="browser-label">{profile.browser_type}</span>
              </span>
            </td>

            <!-- Proxy -->
            <td class="td-proxy">
              {#if proxy}
                <div class="proxy-info">
                  {#if proxyChip(proxy)}
                    <span class="country-chip">{proxyChip(proxy)}</span>
                  {/if}
                  <span class="proxy-name">{proxy.name}</span>
                  <span class="proxy-type">{proxyTypeName(proxy)}</span>
                </div>
              {:else}
                <span class="no-proxy">
                  <Icon name="wifi-off" size={11} />
                  {$t('table_no_proxy')}
                </span>
              {/if}
            </td>

            <!-- Tags -->
            <td class="td-tags" onclick={(e) => e.stopPropagation()}>
              {#each (profile.tags ?? []) as tag}
                {@const color = tagColorMap.get(tag)}
                <span class="tag-chip tinted" style:--chip={color}>
                  {tag}
                </span>
              {/each}
            </td>

            <!-- Last Launch -->
            <td class="td-date">
              {profile.last_launch_at ? formatDateTime(profile.last_launch_at, $locale) : '—'}
            </td>

            <!-- Actions -->
            <td class="td-actions" onclick={(e) => e.stopPropagation()}>
              {#if isRunning}
                <button class="act-btn act-stop" onclick={(e) => stop(e, profile)} disabled={isLoading} title={$t('panel_btn_stop')} aria-label={$t('panel_btn_stop')}>
                  <Icon name="square" size={12} />
                </button>
              {:else}
                <button class="act-btn act-launch" onclick={(e) => launch(e, profile)} disabled={isLoading} title={$t('panel_btn_launch')} aria-label={$t('panel_btn_launch')}>
                  <Icon name="play" size={12} />
                </button>
              {/if}
              <button class="act-btn" onclick={(e) => { e.stopPropagation(); clone(e, profile); }} disabled={isLoading} title={$t('panel_btn_clone')} aria-label={$t('panel_btn_clone')}>
                <Icon name="copy" size={12} />
              </button>
              <button class="act-btn" onclick={(e) => { e.stopPropagation(); onEdit(profile); }} title={$t('panel_btn_edit')} aria-label={$t('panel_btn_edit')}>
                <Icon name="pencil" size={12} />
              </button>
            </td>
          </tr>
        {:else}
          <tr><td colspan="7" class="td-empty">{$t('table_empty')}</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>

<style>
  .table-view { display: flex; flex-direction: column; gap: var(--sp-3); height: 100%; }

  /* панель фильтров — единые примитивы .filter-bar/.search-field/.filter-select/.filter-clear/.count-badge из base.css */

  /* ── Table ── (data-table look: surface card, caps header, subtle row borders) */
  .table-wrap {
    overflow: auto; flex: 1;
    border-radius: var(--radius-lg); border: 1px solid var(--border);
    background: var(--surface);
  }

  table { width: 100%; border-collapse: collapse; font-size: var(--fs-sm); }

  thead {
    background: var(--surface); position: sticky; top: 0; z-index: 1;
    border-bottom: 1px solid var(--border);
  }
  thead th {
    padding: var(--sp-3) var(--sp-3); text-align: left;
    color: var(--text-3); font-weight: var(--fw-bold); font-size: var(--fs-2xs);
    text-transform: uppercase; letter-spacing: 0.7px; white-space: nowrap;
  }
  .th-num { width: 52px; }
  .th-actions { width: 112px; text-align: right; }

  tbody tr {
    border-bottom: 1px solid var(--surface-2);
    cursor: pointer; transition: background 0.12s;
  }
  tbody tr:last-child { border-bottom: none; }
  tbody tr:hover { background: var(--surface-row-hover); }
  tbody tr.running { background: var(--success-bg); }

  td { padding: var(--sp-3) var(--sp-3); vertical-align: middle; }

  /* # column */
  .td-num { width: 52px; display: flex; align-items: center; gap: 0.4rem; }
  .row-num {
    font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-3);
    min-width: 18px; text-align: right;
  }
  .status-dot {
    width: 6px; height: 6px; border-radius: 50%;
    background: var(--border-2); flex-shrink: 0; transition: background 0.2s;
  }
  .status-dot.active {
    background: var(--success-text);
    box-shadow: 0 0 0 2px var(--success-bg);
  }

  /* Name */
  .td-name { white-space: nowrap; max-width: 200px; overflow: hidden; text-overflow: ellipsis; }
  .profile-name { font-weight: var(--fw-semibold); font-size: 0.9rem; color: var(--text); }
  .running-pill {
    display: inline-flex; align-items: center; gap: 0.2rem;
    margin-left: 0.4rem; font-size: var(--fs-2xs); font-weight: var(--fw-semibold);
    color: var(--success-text); background: var(--success-bg);
    padding: 0.1rem 0.45rem; border-radius: var(--radius-sm);
  }

  /* OS — mono chip */
  .td-os { white-space: nowrap; }
  .os-chip {
    display: inline-flex; align-items: center; gap: 5px;
    padding: 4px 10px; border-radius: 7px;
    background: var(--surface-2); font-family: var(--font-mono);
    font-size: 0.78rem; color: var(--text-body);
  }
  .os-icon { font-size: var(--fs-base); }
  .os-label { color: var(--text-body); }
  .sep { color: var(--text-3); }
  .browser-label { color: var(--text-2); text-transform: capitalize; }

  /* Proxy */
  .td-proxy { white-space: nowrap; max-width: 200px; }
  .proxy-info { display: flex; align-items: center; gap: 0.35rem; }
  .country-chip {
    font-size: var(--fs-2xs); font-weight: var(--fw-bold);
    font-family: var(--font-mono);
    background: var(--surface-2); border: 1px solid var(--border);
    padding: 0.1rem 0.4rem; border-radius: var(--radius-sm); color: var(--text-2);
    letter-spacing: 0.03em;
  }
  .proxy-name { color: var(--text); max-width: 100px; overflow: hidden; text-overflow: ellipsis; }
  .proxy-type {
    font-family: var(--font-mono);
    font-size: var(--fs-2xs); font-weight: var(--fw-semibold); letter-spacing: 0.04em;
    padding: 0.15rem 0.45rem; border-radius: 7px;
    background: var(--surface-2); color: var(--text-2);
  }
  .no-proxy { display: inline-flex; align-items: center; gap: var(--sp-1); color: var(--text-dim); font-size: var(--fs-sm); }

  /* Tags */
  .td-tags { max-width: 180px; display: flex; flex-wrap: wrap; gap: var(--sp-1); align-items: center; }
  .tag-chip {
    display: inline-flex; align-items: center;
    border-width: 1px; border-style: solid;
    border-radius: var(--radius-sm); padding: 0.1rem 0.4rem;
    font-size: var(--fs-2xs); white-space: nowrap;
  }

  /* Date */
  .td-date { color: var(--text-3); font-size: var(--fs-sm); white-space: nowrap; }

  /* Actions */
  .td-actions { text-align: right; white-space: nowrap; }
  .act-btn {
    display: inline-flex; align-items: center; justify-content: center;
    width: 32px; height: 32px;
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius-sm);
    color: var(--text-soft); cursor: pointer; padding: 0; transition: all var(--dur-fast);
  }
  .act-btn:hover { background: var(--surface-hover); border-color: var(--border-2); color: var(--text); }
  .act-btn:disabled { opacity: 0.35; cursor: not-allowed; }
  .act-launch:hover { background: var(--success-bg) !important; border-color: color-mix(in srgb, var(--success) 40%, var(--border)) !important; color: var(--success-text) !important; }
  .act-stop:hover   { background: var(--danger-bg)  !important; border-color: color-mix(in srgb, var(--danger)  40%, var(--border)) !important; color: var(--danger-text)  !important; }

  .td-empty { text-align: center; padding: var(--sp-12); color: var(--text-2); }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { ask } from '$lib/core/ui/confirm.svelte';
  import type { Profile, Proxy, WorkspaceColumn } from '$lib/browser/types';
  import { t, locale } from '$lib/core/i18n';
  import { api, launchUnderWay, leaseHolder, startsEmpty } from '$lib/browser/api';
  import { runningStore } from '$lib/browser/store/running.svelte';
  import ProfileSyncBadge from '$lib/browser/components/ProfileSyncBadge.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { refocusSearch } from '$lib/core/ui/searchField';
  import { pinnedHead } from '$lib/core/pinnedHead';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
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
      // Launched from elsewhere already: the button shows it.
      if (launchUnderWay(err)) return;
      // Leased by another device: let the user override, told when it would start empty.
      const holder = leaseHolder(err);
      const title = startsEmpty(err) ? 'profile_sync_files_elsewhere_confirm' : 'profile_sync_in_use_confirm';
      if (holder !== null && (await ask({ title: $t(title, { device: holder }), variant: 'primary' }))) {
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
      <Icon name="search" size={16} />
      <input type="text" placeholder={$t('kanban_filter_search')} bind:value={search} />
      {#if search}
        <button type="button" class="search-clear" onclick={(e) => { search = ''; refocusSearch(e); }} aria-label={$t('common_clear')}><Icon name="x" size={16} /></button>
      {/if}
    </div>
    <div class="filter-select">
      <CustomSelect
        options={[
          { value: '', label: $t('kanban_filter_runtime') },
          { value: 'running', label: $t('kanban_filter_running') },
          { value: 'stopped', label: $t('kanban_filter_stopped') },
        ]}
        bind:value={filterStatus}
      />
    </div>
    <div class="filter-select">
      <CustomSelect
        options={[{ value: '', label: $t('filter_all_tags') }, ...allTags().map((tag) => ({ value: tag, label: tag }))]}
        bind:value={filterTag}
      />
    </div>
    {#if hasFilter}
      <button class="filter-clear" onclick={clearFilters} title={$t('kanban_clear_filters')} aria-label={$t('kanban_clear_filters')}>
        <Icon name="x" size={12} />
      </button>
    {/if}
    <span class="count-badge">{filtered().length} / {profiles.length}</span>
  </div>

  <!-- Table -->
  <div class="table-wrap" use:pinnedHead>
    <table class="pinned-head">
      <thead class="data-table-head">
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
          {@const launching = runningStore.launching.includes(profile.id)}
          {@const syncing = runningStore.syncing.includes(profile.id)}
          {@const isLoading = loadingId === profile.id || launching}
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
              <div class="tags-wrap">
              {#each (profile.tags ?? []) as tag}
                {@const color = tagColorMap.get(tag)}
                <span class="tag-chip tinted" style:--chip={color} title={tag}>
                  <span class="tag-text">{tag}</span>
                </span>
              {/each}
              </div>
            </td>

            <!-- Last Launch -->
            <td class="td-date">
              {profile.last_launch_at ? formatDateTime(profile.last_launch_at, $locale) : '—'}
            </td>

            <!-- Actions -->
            <td class="td-actions" onclick={(e) => e.stopPropagation()}>
              {#if isRunning}
                <button class="icon-btn act-stop" onclick={(e) => stop(e, profile)} disabled={isLoading} title={$t('panel_btn_stop')} aria-label={$t('panel_btn_stop')}>
                  <Icon name="square" size={12} />
                </button>
              {:else}
                <button class="icon-btn act-launch" onclick={(e) => launch(e, profile)} disabled={isLoading} title={syncing ? $t('profile_launch_syncing_hint') : $t('panel_btn_launch')} aria-label={syncing ? $t('profile_launch_syncing') : $t('panel_btn_launch')}>
                  <Icon name="play" size={12} />
                </button>
              {/if}
              <button class="icon-btn" onclick={(e) => { e.stopPropagation(); clone(e, profile); }} disabled={isLoading} title={$t('panel_btn_clone')} aria-label={$t('panel_btn_clone')}>
                <Icon name="copy" size={12} />
              </button>
              <button class="icon-btn" onclick={(e) => { e.stopPropagation(); onEdit(profile); }} title={$t('common_edit')} aria-label={$t('common_edit')}>
                <Icon name="pencil" size={12} />
              </button>
            </td>
          </tr>
        {:else}
          <tr><td colspan="7"><div class="empty-state"><span class="empty-icon" aria-hidden="true"><Icon name="table" size={22} /></span><p>{$t('table_empty')}</p></div></td></tr>
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

  /* The head: the common .data-table-head, pinned, no background (.pinned-head) */
  table { width: 100%; font-size: var(--fs-sm); }
  thead th { padding: 0 var(--sp-3); text-align: left; white-space: nowrap; }
  .tag-text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .th-num { width: 52px; }
  .th-actions { width: 112px; text-align: right; }

  /* The lines and backgrounds are the cells' (separate borders): cut with them under the pinned head */
  tbody tr { cursor: pointer; }
  tbody td { border-bottom: 1px solid var(--divider); transition: background 0.12s; }
  tbody tr:last-child td { border-bottom: none; }
  tbody tr:hover td { background: var(--surface-row-hover); }
  tbody tr.running td { background: var(--success-bg); }

  td { padding: var(--sp-3) var(--sp-3); vertical-align: middle; }

  /* # column */
  .td-num { width: 52px; display: flex; align-items: center; gap: var(--sp-1-5); }
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
  .profile-name { font-weight: var(--fw-semibold); font-size: var(--fs-base); color: var(--text); }
  .running-pill {
    display: inline-flex; align-items: center; gap: var(--sp-1);
    margin-left: var(--sp-1-5); font-size: var(--fs-xs); font-weight: var(--fw-semibold);
    color: var(--success-text); background: var(--success-bg);
    padding: var(--sp-0-5) var(--sp-2); border-radius: var(--radius-sm);
  }

  /* OS — mono chip */
  .td-os { white-space: nowrap; }
  .os-chip {
    display: inline-flex; align-items: center; gap: var(--sp-1);
    padding: var(--sp-1) var(--sp-2-5); border-radius: var(--radius-sm);
    background: var(--surface-2); font-family: var(--font-mono);
    font-size: var(--fs-sm); color: var(--text-body);
  }
  .os-icon { font-size: var(--fs-base); }
  .os-label { color: var(--text-body); }
  .sep { color: var(--text-3); }
  .browser-label { color: var(--text-2); text-transform: capitalize; }

  /* Proxy */
  .td-proxy { white-space: nowrap; max-width: 200px; }
  .proxy-info { display: flex; align-items: center; gap: var(--sp-1-5); }
  .country-chip {
    font-size: var(--fs-xs); font-weight: var(--fw-bold);
    font-family: var(--font-mono);
    background: var(--surface-2); border: 1px solid var(--border);
    padding: var(--sp-0-5) var(--sp-1-5); border-radius: var(--radius-sm); color: var(--text-2);
  }
  .proxy-name { color: var(--text); max-width: 100px; overflow: hidden; text-overflow: ellipsis; }
  .proxy-type {
    font-family: var(--font-mono);
    font-size: var(--fs-xs); font-weight: var(--fw-semibold);
    padding: var(--sp-0-5) var(--sp-2); border-radius: var(--radius-sm);
    background: var(--surface-2); color: var(--text-2);
  }
  .no-proxy { display: inline-flex; align-items: center; gap: var(--sp-1); color: var(--text-dim); font-size: var(--fs-sm); }

  /* Tags */
  .tags-wrap { max-width: 180px; display: flex; flex-wrap: wrap; gap: var(--sp-1); align-items: center; }
  .tag-chip {
    max-width: 100%; min-width: 0;
    display: inline-flex; align-items: center;
    border-width: 1px; border-style: solid;
    border-radius: var(--radius-pill); padding: var(--sp-0-5) var(--sp-1-5);
    font-size: var(--fs-xs); white-space: nowrap;
  }

  /* Date */
  .td-date { color: var(--text-3); font-size: var(--fs-sm); white-space: nowrap; }

  /* Actions */
  .td-actions { text-align: right; white-space: nowrap; }
  /* The plain .icon-btn; launch and stop light their own ink under the pointer. */
  .act-launch:hover:not(:disabled) { color: var(--success-text); }
  .act-stop:hover:not(:disabled) { color: var(--danger-text); }

</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { tick } from 'svelte';
  import { t, locale, type TranslationKey } from '$lib/core/i18n';
  import { api, leaseHolder } from '$lib/browser/api';
  import ProfileSyncBadge from '$lib/browser/components/ProfileSyncBadge.svelte';
  import type { Profile, Proxy, WorkspaceColumn } from '$lib/browser/types';
  import { proxyChip } from '$lib/browser/proxy-label';
  import Icon from '$lib/core/Icon.svelte';
  import Drawer from '$lib/core/ui/Drawer.svelte';
  import Modal from '$lib/core/ui/Modal.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import ExportProfileModal from '$lib/browser/components/ExportProfileModal.svelte';
  import { registry } from '$lib/core/registry';
  import { explainError } from '$lib/browser/tor-error';
  import { formatError, relTime as fmtRelTime, formatDateTime } from '$lib/core/utils';

  interface Props {
    profile: Profile;
    proxy: Proxy | null;
    workspaceId: string;
    columns: WorkspaceColumn[];
    isRunning: boolean;
    onclose: () => void;
    onchange: () => void;
    onsync: (p: Profile) => void;
    onedit: (p: Profile) => void;
    onrawdata: (p: Profile) => void;
  }

  let { profile, proxy, workspaceId, columns, isRunning, onclose, onchange, onsync, onedit, onrawdata }: Props = $props();

  // The other modules' tabs for a profile — TOTP, passwords, notes, SSH — in their declared order.
  const tabs = registry.views('profile', 'tab');
  let activeTab = $state('info');
  let tabsEl = $state<HTMLDivElement | null>(null);
  const drawerBase = 440;
  let drawerWidth = $state(drawerBase);

  // Grow the drawer just enough for the tab labels and their counts.
  $effect(() => {
    for (const tab of tabs) tab.count?.(profile.id);
    $locale;
    const bar = tabsEl;
    if (!bar) return;
    const styles = getComputedStyle(bar);
    const pad = parseFloat(styles.paddingLeft) + parseFloat(styles.paddingRight);
    const gap = parseFloat(styles.columnGap || styles.gap) || 0;
    let content = pad;
    const buttons = [...bar.children] as HTMLElement[];
    for (const button of buttons) content += button.getBoundingClientRect().width;
    content += gap * Math.max(0, buttons.length - 1);
    drawerWidth = Math.min(640, Math.max(drawerBase, Math.ceil(content + 8)));
  });

  const relTime = (iso: string): string => fmtRelTime(iso, $locale);
  let actionLoading = $state(false);
  let deleteModal = $state(false);
  let exportModal = $state(false);
  let error = $state('');
  let cookieFileInput: HTMLInputElement | null = $state(null);
  let cookieImportResult = $state<{ count: number; domains: string[] } | null>(null);
  let exportingCookies = $state(false);

  // Column (single tag) assignment
  let tagsLoading = $state(false);
  const tagColorMap = $derived(new Map(columns.map((col) => [col.tag_name, col.color])));

  // Current column tag = first tag that matches a workspace column
  const currentColumnTag = $derived(
    (profile.tags ?? []).find((t) => columns.some((c) => c.tag_name === t)) ?? ''
  );

  async function setColumn(tagName: string) {
    tagsLoading = true;
    try {
      // Keep non-column tags, replace column tag with the new one
      const nonColumnTags = (profile.tags ?? []).filter(
        (t) => !columns.some((c) => c.tag_name === t)
      );
      const updated = tagName ? [...nonColumnTags, tagName] : nonColumnTags;
      await api.profiles.setTags(profile.id, updated);
      onsync({ ...profile, tags: updated });
    } catch (e) { error = formatError(e); }
    finally { tagsLoading = false; }
  }

  const formatDate = (d: string | null) => (d ? formatDateTime(d, $locale) : $t('panel_never'));

  function getOsLabel(preset: string) {
    const map: Record<string, string> = {
      win10: 'Windows 10', win11: 'Windows 11', macos: 'macOS', linux: 'Linux',
    };
    return map[preset] ?? preset;
  }

  /** Set when another device holds the sync lease; offers "launch anyway". */
  let leaseBlockedBy = $state('');

  async function launch(force = false) {
    actionLoading = true; error = ''; leaseBlockedBy = '';
    try {
      await api.profiles.launch(profile.id, force);
      onsync({ ...profile, status: 'running' });
    } catch (e) {
      const holder = leaseHolder(e);
      if (holder !== null) leaseBlockedBy = holder;
      else error = explainError(e, $t);
    }
    finally { actionLoading = false; }
  }

  async function stop() {
    actionLoading = true; error = '';
    try {
      await api.profiles.stop(profile.id);
      onsync({ ...profile, status: 'stopped' });
    } catch (e) { error = formatError(e); }
    finally { actionLoading = false; }
  }

  async function clone() {
    actionLoading = true; error = '';
    try {
      await api.profiles.clone(profile.id);
      onchange();
    } catch (e) { error = formatError(e); }
    finally { actionLoading = false; }
  }

  async function exportCookies() {
    exportingCookies = true;
    error = '';
    try {
      const { save } = await import('@tauri-apps/plugin-dialog');
      const safeName = profile.name.replace(/[^a-z0-9_-]/gi, '_');
      const path = await save({
        defaultPath: `${safeName}_cookies.json`,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      if (!path) return;
      await api.profiles.exportCookiesToFile(profile.id, path);
    } catch (e) {
      error = formatError(e);
    } finally {
      exportingCookies = false;
    }
  }

  async function confirmDelete() {
    try {
      await api.profiles.delete(profile.id);
      deleteModal = false;
      await tick();
      onchange();
    } catch (e) { error = formatError(e); }
  }

  async function onCookieFileSelected(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    error = '';
    cookieImportResult = null;
    try {
      const text = await file.text();
      const result = await api.profiles.importCookies(profile.id, text);
      cookieImportResult = result;
    } catch (e) {
      error = formatError(e);
    } finally {
      if (cookieFileInput) cookieFileInput.value = '';
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
<Drawer open title={profile.name} width="{drawerWidth}px" {onclose}>
  {#snippet actions()}
    <span class="status-badge" class:running={isRunning}>
      {isRunning ? $t('status_running') : $t('status_stopped')}
    </span>
  {/snippet}
  {#snippet subheader()}
    <div class="tab-bar psp-tabs" bind:this={tabsEl}>
      <button class="tab" class:active={activeTab === 'info'} onclick={() => (activeTab = 'info')}>
        <Icon name="info" size={12} /> {$t('panel_tab_info')}
      </button>
      {#each tabs as tab (tab.id)}
        {@const count = tab.count?.(profile.id) ?? 0}
        <button class="tab" class:active={activeTab === tab.id} onclick={() => (activeTab = tab.id)}>
          <Icon name={tab.icon} size={12} /> {$t(tab.title as TranslationKey)}
          {#if count > 0}
            <span class="tab-count">{count}</span>
          {/if}
        </button>
      {/each}
    </div>
  {/snippet}
  <div class="psp-body">
      {#each tabs.filter((tab) => tab.id === activeTab) as tab (tab.id)}
        <tab.component id={profile.id} {workspaceId} />
      {:else}

      {#if error}
        <div class="error-msg" style="margin-bottom:0.5rem">{error}</div>
      {/if}

      <div class="info-section">
        <div class="info-row">
          <span class="info-label">{$t('panel_os')}</span>
          <span class="info-value">
            <Icon name="monitor" size={12} />
            {getOsLabel(profile.fingerprint_preset)} / {profile.browser_type}
          </span>
        </div>

        <div class="info-row">
          <span class="info-label">{$t('panel_proxy')}</span>
          <span class="info-value" class:no-proxy={!proxy}>
            {#if proxy}
              <Icon name="globe" size={12} />
              {proxy.name}
              {#if proxyChip(proxy)}
                <span class="country-badge">{proxyChip(proxy)}</span>
              {/if}
            {:else}
              <Icon name="wifi-off" size={12} />
              {$t('panel_no_proxy')}
            {/if}
          </span>
        </div>

        <div class="info-row">
          <span class="info-label">{$t('profile_field_locale')}</span>
          <span class="info-value"><Icon name="globe" size={12} />{profile.locale}</span>
        </div>

        <div class="info-row">
          <span class="info-label">{$t('profile_field_screen')}</span>
          <span class="info-value">{profile.screen_width}×{profile.screen_height}</span>
        </div>

        <div class="info-row">
          <span class="info-label">{$t('panel_last_activity')}</span>
          <span class="info-value muted">{formatDate(profile.last_launch_at)}</span>
        </div>

        {#if profile.notes}
          <div class="notes-row">
            <Icon name="file-text" size={12} />
            <span>{profile.notes}</span>
          </div>
        {/if}
      </div>

      <!-- Column (tag) assignment -->
      <div class="tags-section">
        <div class="tags-label">{$t('panel_column')}</div>
        <div class="column-select-row">
          {#each columns as col}
            {@const active = currentColumnTag === col.tag_name}
            <button
              class="col-chip tinted"
              class:active
              style:--chip={col.color}
              aria-pressed={active}
              disabled={tagsLoading}
              onclick={() => setColumn(active ? '' : col.tag_name)}
              title={active ? $t('panel_col_unassign') : `${$t('panel_col_assign')}: ${col.name}`}
            >
              {#if active}<Icon name="check" size={12} />{/if}
              {col.name}
            </button>
          {/each}
          {#if columns.length === 0}
            <span class="no-tags">{$t('panel_no_columns')}</span>
          {/if}
        </div>
      </div>

      <ProfileSyncBadge profileId={profile.id} />

      {#if leaseBlockedBy}
        <div class="error-msg lease-block">
          <span>{$t('profile_sync_in_use', { device: leaseBlockedBy })}</span>
          <button class="btn btn-ghost btn-sm" disabled={actionLoading} onclick={() => launch(true)}>{$t('profile_sync_launch_anyway')}</button>
        </div>
      {/if}

      <div class="panel-actions">
        {#if !isRunning}
          <button class="btn btn-success btn-main" disabled={actionLoading} onclick={() => launch()}>
            <Icon name="play" size={16} />{actionLoading ? '…' : $t('panel_btn_launch')}
          </button>
        {:else}
          <button class="btn btn-danger btn-main" disabled={actionLoading} onclick={stop}>
            <Icon name="square" size={16} />{actionLoading ? '…' : $t('panel_btn_stop')}
          </button>
        {/if}

        <button class="btn btn-ghost" onclick={() => onedit(profile)}>
          <Icon name="pencil" size={13} />{$t('panel_btn_edit')}
        </button>
        <button class="btn btn-ghost" onclick={() => onrawdata(profile)}>
          <Icon name="code" size={13} />{$t('panel_btn_raw_data')}
        </button>
        <button class="btn btn-ghost" disabled={actionLoading} onclick={clone}>
          <Icon name="copy" size={13} />{$t('panel_btn_clone')}
        </button>

        <button class="btn btn-ghost" onclick={() => (exportModal = true)}>
          <Icon name="upload" size={13} />{$t('panel_btn_export')}
        </button>

        <input
          bind:this={cookieFileInput}
          type="file"
          accept=".json,.txt"
          style="display:none"
          onchange={onCookieFileSelected}
        />
        <button
          class="btn btn-ghost"
          disabled={isRunning}
          title={isRunning ? $t('panel_cookie_import_blocked') : $t('panel_cookie_import_hint')}
          onclick={() => cookieFileInput?.click()}
        >
          <Icon name="cookie" size={13} />{$t('panel_btn_import_cookies')}
        </button>

        <button
          class="btn btn-ghost"
          disabled={exportingCookies}
          title={$t('panel_btn_export_cookies_hint')}
          onclick={exportCookies}
        >
          <Icon name="download" size={13} />{exportingCookies ? '…' : $t('panel_btn_export_cookies')}
        </button>

        <button
          class="btn btn-ghost btn-delete"
          onclick={() => (deleteModal = true)}
        >
          <Icon name="trash-2" size={13} />{$t('panel_btn_delete')}
        </button>
      </div>

      {/each}
  </div>
</Drawer>

<Modal
  open={deleteModal}
  title={$t('profiles_btn_delete')}
  message={$t('profiles_confirm_delete', { name: profile.name })}
  confirmLabel={$t('profiles_btn_delete')}
  cancelLabel={$t('profile_btn_cancel')}
  variant="danger"
  onconfirm={confirmDelete}
  oncancel={() => (deleteModal = false)}
/>

<ExportProfileModal
  {profile}
  {proxy}
  open={exportModal}
  onclose={() => (exportModal = false)}
/>

<Dialog open={!!cookieImportResult} title={$t('cookies_imported_title')} width="340px" onclose={() => (cookieImportResult = null)}>
  {#if cookieImportResult}
    <div class="crm-count">{$t('cookies_imported_count', { n: String(cookieImportResult.count) })}</div>
    {#if cookieImportResult.domains.length > 0}
      <div class="crm-domains-label">{$t('cookies_imported_domains', { n: cookieImportResult.domains.length + (cookieImportResult.domains.length === 20 ? '+' : '') })}</div>
      <div class="crm-domains">
        {#each cookieImportResult.domains as d}
          <span class="crm-domain">{d}</span>
        {/each}
      </div>
    {/if}
  {/if}
  {#snippet footer()}
    <button class="btn btn-ghost" onclick={() => (cookieImportResult = null)}>OK</button>
  {/snippet}
</Dialog>

<style>
  .status-badge {
    display: inline-flex; align-items: center; gap: 6px;
    font-size: 0.72rem; font-weight: var(--fw-bold); padding: 4px 11px;
    border-radius: var(--radius-sm); text-transform: uppercase; letter-spacing: 0.4px;
    background: var(--surface-2); color: var(--text-2);
    border: 1px solid var(--border-2);
    width: fit-content;
  }
  .status-badge::before { content: ''; width: 6px; height: 6px; border-radius: 50%; background: currentColor; }
  .status-badge.running { background: var(--success-bg); color: var(--success-text); border-color: var(--success-border); }
  .lease-block { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; margin-bottom: 0.5rem; }

  .psp-body {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    min-height: 100%;
  }

  .info-section { display: flex; flex-direction: column; gap: 14px; }

  /* Design: grid 135px / 1fr rows */
  .info-row {
    display: grid;
    grid-template-columns: 135px 1fr;
    align-items: center;
    gap: var(--sp-3);
    font-size: var(--fs-base);
  }

  .info-label { color: var(--text-faint); font-size: 0.82rem; }

  .info-value {
    color: var(--text);
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .info-value :global(svg) { color: var(--text-2); }
  .info-value.no-proxy { color: var(--text-dim); }
  .info-value.muted { color: var(--text-2); }

  .country-badge {
    font-size: var(--fs-2xs);
    background: var(--surface-2);
    border: 1px solid var(--border);
    padding: 0 0.35rem;
    border-radius: 999px;
    color: var(--text-2);
  }

  .notes-row {
    display: flex;
    align-items: flex-start;
    gap: 0.4rem;
    font-size: var(--fs-base);
    color: var(--text-2);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--sp-2) 0.65rem;
  }

  .psp-tabs { overflow: visible; }
  .psp-tabs :global(.tab) { flex-shrink: 0; }

  .panel-actions {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .panel-actions .btn {
    width: 100%;
    min-width: 0;
    justify-content: flex-start;
    height: 36px;
    padding: 0 12px;
    border-radius: var(--radius);
    font-size: 0.82rem;
    white-space: nowrap;
    overflow: hidden;
  }
  .panel-actions .btn-main,
  .panel-actions .btn-delete {
    grid-column: 1 / -1;
    justify-content: center;
  }
  .panel-actions .btn-main { height: 42px; font-size: 0.92rem; font-weight: var(--fw-bold); }
  .btn-delete { color: var(--danger-text) !important; }
  .btn-delete:hover:not(:disabled) { background: var(--danger-bg) !important; border-color: var(--danger-border) !important; }

  .crm-count {
    font-size: var(--fs-xl);
    font-weight: 800;
    color: var(--text);
    line-height: 1;
    margin-bottom: 0.65rem;
  }

  .crm-domains-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-3);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: var(--sp-1);
  }

  .crm-domains {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1);
    max-height: 120px;
    overflow-y: auto;
  }

  .crm-domain {
    font-size: var(--fs-xs);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.1rem 0.4rem;
    color: var(--text-2);
    font-family: var(--font-mono);
  }

  /* Tags */
  .tags-section { display: flex; flex-direction: column; gap: 10px; }
  .tags-label { font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--text-3); text-transform: uppercase; letter-spacing: 0.8px; }
  .no-tags { font-size: var(--fs-sm); color: var(--text-2); }
  .column-select-row { display: flex; flex-wrap: wrap; gap: 0.4rem; }
  /* Colours: the tinted rule of base.css (`.active`: the pressed tint, a
     border and ring in the ink). The picked column also has a check, so it
     does not hang on colour alone. An unpicked column stays readable: no
     opacity. */
  .col-chip {
    display: inline-flex; align-items: center; gap: var(--sp-1);
    border-width: 1px; border-style: solid;
    border-radius: 9px; padding: 5px 13px;
    font-size: var(--fs-sm); font-weight: var(--fw-semibold);
    cursor: pointer; transition: all 0.15s;
  }
  .col-chip:disabled { cursor: not-allowed; }
</style>

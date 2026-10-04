<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The Camoufox card of the settings page: install state, download, update check. -->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { api } from '$lib/browser/api';
  import type { CamoufoxStatus } from '$lib/browser/types';
  import { formatError } from '$lib/core/utils';

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  let camoufox = $state<CamoufoxStatus | null>(null);
  let downloading = $state(false);
  let extracting = $state(false);
  let downloadError = $state('');
  let downloadDone = $state('');
  let progress = $state<{ percent: number; downloaded: number; total: number } | null>(null);
  let latestCamoufox = $state<string | null>(null);
  let checkingUpdate = $state(false);
  let checkUpdateError = $state('');
  let unlisteners: (() => void)[] = [];

  function formatMb(bytes: number) {
    return (bytes / 1024 / 1024).toFixed(0) + ' MB';
  }

  onMount(async () => {
    camoufox = await api.camoufox.status().catch(() => null);

    // Restore state if download was already running
    const dlState = await api.camoufox.downloadState().catch(() => null);
    if (dlState?.state === 'downloading') {
      downloading = true;
      if (dlState.downloaded && dlState.total) {
        progress = { percent: dlState.percent ?? 0, downloaded: dlState.downloaded, total: dlState.total };
      }
    }

    if (isTauri) {
      const { listen } = await import('@tauri-apps/api/event');

      unlisteners.push(await listen<{ state: string; downloaded: number; total: number; percent: number }>(
        'camoufox://progress',
        (e) => {
          downloading = true;
          extracting = false;
          progress = { percent: e.payload.percent, downloaded: e.payload.downloaded, total: e.payload.total };
        }
      ));

      unlisteners.push(await listen('camoufox://extracting', () => {
        extracting = true;
        progress = null;
      }));

      unlisteners.push(await listen<string>('camoufox://done', async (e) => {
        downloading = false;
        extracting = false;
        progress = null;
        downloadDone = e.payload;
        camoufox = await api.camoufox.status().catch(() => null);
        latestCamoufox = null;
      }));

      unlisteners.push(await listen<string>('camoufox://error', (e) => {
        downloading = false;
        extracting = false;
        progress = null;
        if (!e.payload.includes('cancelled')) {
          downloadError = e.payload;
        }
      }));
    }
  });

  onDestroy(() => unlisteners.forEach((fn) => fn()));

  async function downloadCamoufox() {
    downloadError = '';
    downloadDone = '';
    try {
      await api.camoufox.download();
      downloading = true;
    } catch (e) {
      downloadError = formatError(e);
    }
  }

  async function cancelDownload() {
    await api.camoufox.cancel().catch(() => {});
    downloading = false;
    extracting = false;
    progress = null;
  }

  async function checkForUpdates() {
    checkingUpdate = true;
    checkUpdateError = '';
    latestCamoufox = null;
    try {
      latestCamoufox = await api.camoufox.latestVersion();
    } catch (e) {
      checkUpdateError = formatError(e);
    } finally {
      checkingUpdate = false;
    }
  }

  function isUpToDate(): boolean {
    if (!latestCamoufox || !camoufox?.camoufox_tag) return false;
    return latestCamoufox === camoufox.camoufox_tag;
  }
</script>

{#if camoufox === null}
  <p class="muted">{$t('settings_camoufox_checking')}</p>
{:else if camoufox.installed}
  <div class="status-row">
    <span class="badge badge-ok">{$t('settings_camoufox_installed')}</span>
  </div>

  <div class="version-table">
    {#if camoufox.camoufox_tag}
      <div class="version-row">
        <span class="version-label">{$t('settings_camoufox_tag')}</span>
        <span class="version-value">{camoufox.camoufox_tag}</span>
      </div>
    {/if}
    {#if camoufox.version}
      <div class="version-row">
        <span class="version-label">{$t('settings_camoufox_firefox_version')}</span>
        <span class="version-value">{camoufox.version}</span>
      </div>
    {/if}
    {#if camoufox.path}
      <div class="version-row">
        <span class="version-label">{$t('settings_camoufox_path')}</span>
        <span class="version-value path-value">{camoufox.path}</span>
      </div>
    {/if}
  </div>

  <div class="btn-row">
    <button class="btn btn-primary btn-sm" disabled={downloading} onclick={downloadCamoufox}>
      {downloading ? $t('settings_camoufox_btn_updating') : $t('settings_camoufox_btn_update')}
    </button>
    <button class="btn btn-ghost btn-sm" disabled={checkingUpdate || downloading} onclick={checkForUpdates}>
      {checkingUpdate ? $t('settings_camoufox_checking_update') : $t('settings_camoufox_check_update')}
    </button>
    {#if downloading}
      <button class="btn btn-ghost btn-sm" onclick={cancelDownload}>{$t('common_cancel')}</button>
    {/if}
  </div>

  {#if latestCamoufox}
    {#if isUpToDate()}
      <p class="ok-msg">{$t('settings_camoufox_up_to_date')}</p>
    {:else}
      <p class="warn-msg">{$t('settings_camoufox_update_available', { version: latestCamoufox })}</p>
    {/if}
  {/if}
  {#if checkUpdateError}
    <div class="error-msg">{checkUpdateError}</div>
  {/if}
{:else}
  <div class="status-row">
    <span class="badge badge-warn">{$t('settings_camoufox_not_installed')}</span>
  </div>
  <p class="muted small">{$t('settings_camoufox_download_hint')}</p>
  <div class="btn-row">
    <button class="btn btn-primary btn-sm" disabled={downloading} onclick={downloadCamoufox}>
      {downloading ? $t('settings_camoufox_btn_downloading') : $t('settings_camoufox_btn_download')}
    </button>
    {#if downloading}
      <button class="btn btn-ghost btn-sm" onclick={cancelDownload}>{$t('common_cancel')}</button>
    {/if}
  </div>
{/if}

{#if extracting}
  <p class="muted small">{$t('settings_camoufox_extracting')}</p>
{/if}

{#if downloading && progress}
  <div class="progress-wrap">
    <div class="progress-bar">
      <div class="progress-fill" style="width: {progress.percent}%"></div>
    </div>
    <span class="progress-label">
      {formatMb(progress.downloaded)} / {formatMb(progress.total)} · {progress.percent}%
    </span>
  </div>
{/if}

{#if downloadDone}
  <p class="ok-msg">{$t('settings_camoufox_success', { version: downloadDone })}</p>
{/if}
{#if downloadError}
  <div class="error-msg">{downloadError}</div>
{/if}

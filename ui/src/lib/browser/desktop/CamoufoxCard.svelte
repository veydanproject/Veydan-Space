<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The Camoufox section of the settings: what is installed, the download or the update with its progress, the check for a newer release; one primary button, only when there is something to install. -->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { api } from '$lib/browser/api';
  import type { CamoufoxStatus } from '$lib/browser/types';
  import { formatError } from '$lib/core/utils';
  import Icon from '$lib/core/Icon.svelte';

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
    if (dlState?.state === 'extracting') {
      downloading = true;
      extracting = true;
    } else if (dlState?.state === 'downloading') {
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

  /** The newest release upstream is the one this build installs. */
  function latestIsPinned(): boolean {
    return !!latestCamoufox && latestCamoufox === camoufox?.pinned_version;
  }

  /** There is something to install: this Camoufox is not verified, or a check found the release this build pins. */
  function needsUpdate(): boolean {
    return !!camoufox?.update_available || (latestIsPinned() && !isUpToDate());
  }

  /** The errors the backend marks with a code, in the language of the UI. */
  function explainError(message: string): string {
    if (message.includes('camoufox_in_use')) return $t('settings_camoufox_in_use');
    if (message.includes('camoufox_unavailable')) return $t('settings_camoufox_unavailable');
    return message;
  }
</script>

{#snippet note(tone: 'ok' | 'warn' | 'danger' | 'info' | 'quiet', icon: string, text: string)}
  <div class="note {tone}" role={tone === 'danger' ? 'alert' : tone === 'quiet' ? undefined : 'status'}>
    <Icon name={icon} size={18} />
    <span>{text}</span>
  </div>
{/snippet}

<div class="card cf">
  <div class="head">
    <span class="tile" aria-hidden="true"><Icon name="browser" size={22} /></span>
    <span class="who">
      <strong>{$t('settings_camoufox_title')}</strong>
      {#if camoufox === null}
        <span class="sub">{$t('settings_camoufox_checking')}</span>
      {:else if camoufox.installed && camoufox.camoufox_tag}
        <span class="sub mono">{camoufox.camoufox_tag}</span>
      {/if}
    </span>
    {#if camoufox}
      <span class="badge" class:badge-ok={camoufox.installed} class:badge-warn={!camoufox.installed}>
        {camoufox.installed ? $t('settings_camoufox_installed') : $t('settings_camoufox_not_installed')}
      </span>
    {/if}
  </div>

  {#if camoufox?.installed && (camoufox.camoufox_tag || camoufox.version || camoufox.path)}
    <dl class="facts">
      {#if camoufox.camoufox_tag}
        <div><dt>{$t('settings_camoufox_tag')}</dt><dd class="mono">{camoufox.camoufox_tag}</dd></div>
      {/if}
      {#if camoufox.version}
        <div><dt>{$t('settings_camoufox_firefox_version')}</dt><dd class="mono">{camoufox.version}</dd></div>
      {/if}
      {#if camoufox.path}
        <div><dt>{$t('settings_camoufox_path')}</dt><dd class="mono path">{camoufox.path}</dd></div>
      {/if}
    </dl>
  {/if}

  {#if downloading && progress}
    <div class="progress">
      <div class="track">
        <div class="bar" role="progressbar" aria-label={$t('settings_camoufox_title')} aria-valuemin={0} aria-valuemax={100} aria-valuenow={progress.percent}>
          <div class="fill" style="width: {progress.percent}%"></div>
        </div>
        <span class="label mono">{formatMb(progress.downloaded)} / {formatMb(progress.total)} · {progress.percent}%</span>
      </div>
      {#if !extracting}
        <button class="btn btn-ghost" type="button" onclick={cancelDownload}>{$t('common_cancel')}</button>
      {/if}
    </div>
  {/if}

  <!-- The work and its outcome show whatever the status says, even when it could not be read. -->
  <div class="notes">
    {#if extracting}
      <div class="note quiet" role="status"><span class="spinner" aria-hidden="true"></span><span>{$t('settings_camoufox_extracting')}</span></div>
    {/if}
    {#if camoufox !== null}
      {#if camoufox.installed}
        {#if camoufox.update_available}
          {@render note('warn', 'alert-triangle', $t('settings_camoufox_unverified', { version: camoufox.pinned_version }))}
        {/if}
        {#if latestCamoufox}
          {#if isUpToDate() && camoufox.verified}
            {@render note('ok', 'check', $t('settings_camoufox_up_to_date'))}
          {:else if latestIsPinned()}
            {@render note('warn', 'download', $t('settings_camoufox_update_available', { version: latestCamoufox }))}
          {:else if !isUpToDate()}
            {@render note('info', 'info', $t('settings_camoufox_upstream_newer', { version: latestCamoufox }))}
          {/if}
        {/if}
        {#if checkUpdateError}
          {@render note('danger', 'alert-triangle', checkUpdateError)}
        {/if}
      {:else if camoufox.download_available}
        {@render note('quiet', 'download', $t('settings_camoufox_download_hint'))}
      {:else}
        {@render note('warn', 'alert-triangle', $t('settings_camoufox_unavailable'))}
      {/if}
    {/if}
    {#if downloadDone}
      {@render note('ok', 'check', $t('settings_camoufox_success', { version: downloadDone }))}
    {/if}
    {#if downloadError}
      {@render note('danger', 'alert-triangle', explainError(downloadError))}
    {/if}
  </div>

  {#if camoufox !== null}

    <div class="actions">
      {#if downloading && !progress && !extracting}
        <button class="btn btn-ghost" type="button" onclick={cancelDownload}>{$t('common_cancel')}</button>
      {/if}
      {#if camoufox.installed}
        <button class="btn btn-ghost" type="button" disabled={checkingUpdate || downloading} onclick={checkForUpdates}>
          {checkingUpdate ? $t('settings_camoufox_checking_update') : $t('settings_camoufox_check_update')}
        </button>
        <!-- The primary button only when there is something to install: a check found the pinned release, or this one is not verified. -->
        <button class="btn" class:btn-primary={needsUpdate()} class:btn-ghost={!needsUpdate()} type="button" disabled={downloading || !camoufox.download_available} onclick={downloadCamoufox}>
          {downloading ? $t('settings_camoufox_btn_updating') : $t('settings_camoufox_btn_update')}
        </button>
      {:else}
        <button class="btn btn-primary" type="button" disabled={downloading || !camoufox.download_available} onclick={downloadCamoufox}>
          {downloading ? $t('settings_camoufox_btn_downloading') : $t('settings_camoufox_btn_download')}
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .cf { padding: 0; display: flex; flex-direction: column; overflow: hidden; container-type: inline-size; }
  .head { display: flex; align-items: center; gap: var(--sp-4); padding: var(--sp-5) var(--sp-6); }
  .tile {
    display: inline-flex; align-items: center; justify-content: center; width: var(--control-h-lg); height: var(--control-h-lg); flex-shrink: 0;
    border-radius: var(--radius-md); background: var(--accent-tint); color: var(--accent-text);
  }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .who strong { font-size: var(--fs-lg); font-weight: var(--fw-bold); line-height: 1.3; }
  .sub { font-size: var(--fs-sm); color: var(--text-2); }

  /* What is installed: the names in one column, the values in the other; in a narrow window the name over its value. */
  .facts { margin: 0; border-top: 1px solid var(--divider); }
  .facts div { display: flex; align-items: baseline; gap: var(--sp-4); padding: var(--sp-3) var(--sp-6); }
  .facts div + div { border-top: 1px solid var(--divider); }
  .facts dt { flex: 0 0 120px; font-size: var(--fs-base); color: var(--text-2); }
  .facts dd { flex: 1; min-width: 0; margin: 0; font-size: var(--fs-sm); color: var(--text); }
  .path { overflow-wrap: anywhere; }
  @container (max-width: 560px) {
    .facts div { flex-direction: column; gap: 2px; }
    .facts dt { flex: none; }
  }

  .progress { display: flex; align-items: center; gap: var(--sp-4); padding: var(--sp-4) var(--sp-6); border-top: 1px solid var(--divider); }
  .track { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .bar { height: 8px; border-radius: var(--radius-pill); background: var(--surface-2); overflow: hidden; }
  .fill { height: 100%; border-radius: var(--radius-pill); background: var(--accent); transition: width var(--dur-base); }
  .label { font-size: var(--fs-xs); color: var(--text-2); }

  .notes { display: flex; flex-direction: column; gap: var(--sp-2); padding: 0 var(--sp-6); }
  .notes > :last-child { margin-bottom: var(--sp-4); }
  .note {
    display: flex; align-items: flex-start; gap: var(--sp-3); padding: var(--sp-3) var(--sp-4);
    border-radius: var(--radius-md); font-size: var(--fs-base); line-height: 1.5; color: var(--text);
  }
  .note :global(svg), .note .spinner { flex-shrink: 0; margin-top: 2px; }
  .note.ok { background: var(--success-bg); }
  .note.ok :global(svg) { color: var(--success-text); }
  .note.warn { background: var(--warn-bg); }
  .note.warn :global(svg) { color: var(--warn-text); }
  .note.danger { background: var(--danger-bg); }
  .note.danger :global(svg) { color: var(--danger-text); }
  .note.info { background: var(--accent-tint); }
  .note.info :global(svg) { color: var(--accent-text); }
  .note.quiet { background: var(--surface-2); color: var(--text-2); }

  .actions { display: flex; justify-content: flex-end; flex-wrap: wrap; gap: var(--sp-2); padding: var(--sp-4) var(--sp-6) var(--sp-5); border-top: 1px solid var(--divider); }
  @container (max-width: 560px) { .actions .btn { flex: 1; justify-content: center; } }
</style>

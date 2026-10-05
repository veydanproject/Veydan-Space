<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The install block of Tor: state, download with progress, install from an
  archive, update, removal and the install folder. Shown by the settings card
  and by the Tor page; the phone has no Tor daemon of its own and never mounts it.
-->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { api } from '$lib/tor/api';
  import type { TorStatus } from '$lib/tor/types';
  import { formatError } from '$lib/core/utils';
  import { torErrorText } from '$lib/tor/settings';
  import { ask } from '$lib/core/ui/confirm.svelte';
  import { clampPercent, isCancelled, progressLine, versionLine } from '$lib/tor/format';

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  let status = $state<TorStatus | null>(null);
  let downloading = $state(false);
  let extracting = $state(false);
  let installing = $state(false);
  let removing = $state(false);
  let error = $state('');
  let done = $state('');
  let progress = $state<{ percent: number; downloaded: number; total: number } | null>(null);
  let unlisteners: (() => void)[] = [];

  const busy = $derived(downloading || installing || removing);

  async function refresh() {
    status = await api.tor.status().catch(() => null);
  }

  onMount(async () => {
    await refresh();

    // Restore state if the download was already running
    const dl = await api.tor.downloadState().catch(() => null);
    if (dl?.state === 'downloading') {
      downloading = true;
      if (dl.downloaded && dl.total) {
        progress = { percent: dl.percent ?? 0, downloaded: dl.downloaded, total: dl.total };
      }
    }

    if (isTauri) {
      const { listen } = await import('@tauri-apps/api/event');

      unlisteners.push(await listen<{ downloaded: number; total: number; percent: number }>('tor://progress', (e) => {
        downloading = true;
        extracting = false;
        progress = { percent: e.payload.percent, downloaded: e.payload.downloaded, total: e.payload.total };
      }));

      unlisteners.push(await listen('tor://extracting', () => {
        extracting = true;
        progress = null;
      }));

      unlisteners.push(await listen<string>('tor://done', async (e) => {
        downloading = false;
        extracting = false;
        progress = null;
        done = e.payload ? $t('tor_success', { version: e.payload }) : $t('tor_success_unknown');
        await refresh();
      }));

      unlisteners.push(await listen<string>('tor://error', (e) => {
        downloading = false;
        extracting = false;
        progress = null;
        if (!isCancelled(e.payload)) error = torErrorText(e.payload, (k) => $t(k));
      }));
    }
  });

  onDestroy(() => unlisteners.forEach((fn) => fn()));

  async function download() {
    error = '';
    done = '';
    try {
      await api.tor.download();
      downloading = true;
    } catch (e) {
      error = torErrorText(formatError(e), (k) => $t(k));
    }
  }

  async function cancel() {
    await api.tor.cancel().catch(() => {});
    downloading = false;
    extracting = false;
    progress = null;
  }

  async function pickArchive(): Promise<string | null> {
    if (!isTauri) return null;
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      // The dialog matches the last extension only: `.tar.gz` is `gz`.
      const selected = await open({
        multiple: false,
        title: $t('tor_archive_title'),
        filters: [{ name: $t('tor_archive_filter'), extensions: ['gz', 'tgz'] }],
      });
      return typeof selected === 'string' ? selected : null;
    } catch {
      return null;
    }
  }

  async function installFromArchive() {
    const path = await pickArchive();
    if (!path) return;
    error = '';
    done = '';
    installing = true;
    try {
      let result = await api.tor.installFromArchive(path, false);
      if (result.state === 'unknown_hash') {
        const yes = await ask({
          title: $t('tor_unknown_hash_title'),
          message: $t('tor_unknown_hash_message', { hash: result.sha256 }),
          confirmLabel: $t('tor_unknown_hash_confirm'),
        });
        if (!yes) return;
        result = await api.tor.installFromArchive(path, true);
      }
      if (result.state === 'installed') {
        done = result.version ? $t('tor_success', { version: result.version }) : $t('tor_success_unknown');
      }
      await refresh();
    } catch (e) {
      error = torErrorText(formatError(e), (k) => $t(k));
    } finally {
      installing = false;
    }
  }

  async function remove() {
    const yes = await ask({
      title: $t('tor_remove_title'),
      message: $t('tor_remove_message'),
      confirmLabel: $t('tor_btn_remove'),
    });
    if (!yes) return;
    error = '';
    done = '';
    removing = true;
    try {
      await api.tor.remove();
      await refresh();
    } catch (e) {
      error = torErrorText(formatError(e), (k) => $t(k));
    } finally {
      removing = false;
    }
  }
</script>

<div class="tor-install">
  {#if status === null}
    <p class="muted">{$t('tor_checking')}</p>
  {:else}
    <div class="status-row">
      {#if status.installed}
        <span class="badge badge-ok">{$t('tor_installed')}</span>
        {#if status.source === 'manual'}
          <span class="badge badge-warn">{$t('tor_installed_by_hand')}</span>
        {/if}
      {:else}
        <span class="badge badge-warn">{$t('tor_not_installed')}</span>
      {/if}
    </div>

    {#if status.installed}
      <div class="version-table">
        {#if status.version || status.tor_version}
          <div class="version-row">
            <span class="version-label">{$t('tor_version')}</span>
            <span class="version-value">{versionLine(status.version, status.tor_version)}</span>
          </div>
        {/if}
        {#if status.path}
          <div class="version-row">
            <span class="version-label">{$t('tor_path')}</span>
            <span class="version-value path-value">{status.path}</span>
          </div>
        {/if}
      </div>
      {#if status.update_available}
        <p class="warn-msg">{$t('tor_update_available', { version: status.pinned_version })}</p>
      {/if}
    {:else}
      <p class="muted small">{$t('tor_download_hint')}</p>
    {/if}

    <div class="btn-row">
      {#if !status.installed}
        <button class="btn btn-primary btn-sm" disabled={busy} onclick={download}>
          {downloading ? $t('tor_btn_downloading') : $t('tor_btn_download')}
        </button>
      {:else if status.update_available || downloading}
        <button class="btn btn-primary btn-sm" disabled={busy} onclick={download}>
          {downloading ? $t('tor_btn_updating') : $t('tor_btn_update')}
        </button>
      {/if}
      {#if downloading}
        <button class="btn btn-ghost btn-sm" onclick={cancel}>{$t('common_cancel')}</button>
      {/if}
      <button class="btn btn-ghost btn-sm" disabled={busy} onclick={installFromArchive}>
        {installing ? $t('tor_btn_installing') : $t('tor_btn_archive')}
      </button>
      {#if status.installed}
        <button class="btn btn-ghost btn-sm" disabled={busy} onclick={remove}>{$t('tor_btn_remove')}</button>
      {/if}
    </div>

    {#if extracting}
      <p class="muted small">{$t('tor_extracting')}</p>
    {/if}

    {#if downloading && progress}
      <div class="progress-wrap">
        <div class="progress-bar">
          <div class="progress-fill" style="width: {clampPercent(progress.percent)}%"></div>
        </div>
        <span class="progress-label">{progressLine(progress)}</span>
      </div>
    {/if}

    {#if done}
      <p class="ok-msg">{done}</p>
    {/if}
    {#if error}
      <div class="error-msg">{error}</div>
    {/if}

    <div class="version-table">
      <div class="version-row">
        <span class="version-label">{$t('tor_dir_label')}</span>
        <span class="version-value path-value">{status.install_dir}</span>
      </div>
    </div>
    <p class="muted small">{$t('tor_dir_hint')}</p>
  {/if}
</div>

<style>
  /* The settings page styles these classes for its own cards only; the Tor page
     shows the same block, so the block carries its own. */
  .tor-install { display: flex; flex-direction: column; gap: var(--sp-4); }
  .status-row, .btn-row { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; }
  .muted { font-size: var(--fs-base); }
  .small { font-size: var(--fs-sm); }
  .ok-msg { font-size: var(--fs-sm); color: var(--success-text); }
  .warn-msg { font-size: var(--fs-sm); color: var(--warn-text); }
  .error-msg { font-size: var(--fs-sm); color: var(--danger-text); }
  .version-table { display: flex; flex-direction: column; gap: 0.35rem; }
  .version-row { display: flex; align-items: baseline; gap: var(--sp-2); }
  .version-label { font-size: 0.82rem; color: var(--text-faint); min-width: 120px; flex-shrink: 0; }
  .version-value { font-size: var(--fs-sm); color: var(--text-body); font-family: var(--font-mono); }
  .path-value { font-size: var(--fs-2xs); word-break: break-all; }
  .progress-wrap { display: flex; flex-direction: column; gap: 0.3rem; }
  .progress-bar { height: 5px; background: var(--surface-2); border-radius: 999px; overflow: hidden; }
  .progress-fill { height: 100%; background: var(--accent); border-radius: 999px; transition: width 0.2s ease; }
  .progress-label { font-size: var(--fs-2xs); color: var(--text-2); font-family: var(--font-mono); }
</style>

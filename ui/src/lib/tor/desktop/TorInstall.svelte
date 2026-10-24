<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The install block of Tor: state, download with progress, install from an
  archive, update, removal and the install folder: the tab «Установка» of the
  Tor section of the settings, which tells `onstatus` what it reads for its
  head; the phone has no Tor daemon of its own and never mounts it.
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
  import TorNote from '$lib/tor/desktop/TorNote.svelte';

  let { onstatus }: { onstatus?: (status: TorStatus | null) => void } = $props();

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
    onstatus?.(status);
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

<div class="card install">
  {#if status === null}
    <div class="notes"><TorNote wait>{$t('tor_checking')}</TorNote></div>
  {:else}
    <dl class="facts">
      {#if status.installed && (status.version || status.tor_version)}
        <div><dt>{$t('tor_version')}</dt><dd class="mono">{versionLine(status.version, status.tor_version)}</dd></div>
      {/if}
      {#if status.installed && status.path}
        <div><dt>{$t('tor_path')}</dt><dd class="mono path">{status.path}</dd></div>
      {/if}
      <div><dt>{$t('tor_dir_label')}</dt><dd class="mono path">{status.install_dir}</dd></div>
    </dl>

    {#if downloading && progress}
      <div class="progress">
        <div class="track">
          <div class="bar" role="progressbar" aria-label={$t('settings_tor_title')} aria-valuemin={0} aria-valuemax={100} aria-valuenow={clampPercent(progress.percent)}>
            <div class="fill" style="width: {clampPercent(progress.percent)}%"></div>
          </div>
          <span class="label mono">{progressLine(progress)}</span>
        </div>
        <button class="btn btn-ghost" type="button" onclick={cancel}>{$t('common_cancel')}</button>
      </div>
    {/if}

    <div class="notes">
      {#if extracting}<TorNote wait>{$t('tor_extracting')}</TorNote>{/if}
      {#if status.installed && status.update_available}
        <TorNote tone="warn" icon="download">{$t('tor_update_available', { version: status.pinned_version })}</TorNote>
      {/if}
      {#if !status.installed}
        <TorNote icon="download">{$t('tor_download_hint')}</TorNote>
      {/if}
      {#if done}<TorNote tone="ok">{done}</TorNote>{/if}
      {#if error}<TorNote tone="danger">{error}</TorNote>{/if}
      <TorNote icon="folder">{$t('tor_dir_hint')}</TorNote>
    </div>

    <!-- The removal stands apart at the left; the way to install at the right, the primary one only when there is something to get. -->
    <div class="actions">
      {#if status.installed}
        <button class="btn btn-ghost remove" type="button" disabled={busy} onclick={remove}>{$t('tor_btn_remove')}</button>
      {/if}
      <span class="spacer"></span>
      {#if downloading && !progress}
        <button class="btn btn-ghost" type="button" onclick={cancel}>{$t('common_cancel')}</button>
      {/if}
      <button class="btn btn-ghost" type="button" disabled={busy} onclick={installFromArchive}>
        {installing ? $t('tor_btn_installing') : $t('tor_btn_archive')}
      </button>
      {#if !status.installed}
        <button class="btn btn-primary" type="button" disabled={busy} onclick={download}>
          {downloading ? $t('tor_btn_downloading') : $t('tor_btn_download')}
        </button>
      {:else if status.update_available || downloading}
        <button class="btn btn-primary" type="button" disabled={busy} onclick={download}>
          {downloading ? $t('tor_btn_updating') : $t('tor_btn_update')}
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .install { padding: 0; display: flex; flex-direction: column; overflow: hidden; container-type: inline-size; }
  .facts { margin: 0; }
  .facts div { display: flex; align-items: baseline; gap: var(--sp-4); padding: var(--sp-3) var(--sp-5); }
  .facts div + div { border-top: 1px solid var(--divider); }
  .facts dt { flex: 0 0 150px; font-size: var(--fs-base); color: var(--text-2); }
  .facts dd { flex: 1; min-width: 0; margin: 0; font-size: var(--fs-sm); color: var(--text); }
  .path { overflow-wrap: anywhere; }
  @container (max-width: 560px) {
    .facts div { flex-direction: column; gap: 2px; }
    .facts dt { flex: none; }
  }
  .progress { display: flex; align-items: center; gap: var(--sp-4); padding: var(--sp-4) var(--sp-5); border-top: 1px solid var(--divider); }
  .track { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .bar { height: 8px; border-radius: var(--radius-pill); background: var(--surface-2); overflow: hidden; }
  .fill { height: 100%; border-radius: var(--radius-pill); background: var(--accent); transition: width var(--dur-base); }
  .label { font-size: var(--fs-xs); color: var(--text-2); }
  .notes { display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-3) var(--sp-5) var(--sp-4); border-top: 1px solid var(--divider); }
  .actions { display: flex; align-items: center; flex-wrap: wrap; gap: var(--sp-2); padding: var(--sp-4) var(--sp-5); border-top: 1px solid var(--divider); }
  .spacer { flex: 1; }
  .remove { color: var(--danger-text); }
</style>

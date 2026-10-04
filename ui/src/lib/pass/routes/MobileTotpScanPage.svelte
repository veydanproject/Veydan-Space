<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { goto } from '$app/navigation';
  import {
    scan,
    cancel,
    checkPermissions,
    requestPermissions,
    openAppSettings,
    Format,
  } from '@tauri-apps/plugin-barcode-scanner';
  import Icon from '$lib/core/Icon.svelte';
  import { formatError } from '$lib/core/utils';
  import { t } from '$lib/core/mobile/i18n';

  let error = $state('');
  let denied = $state(false);
  let active = false;

  async function start() {
    let perm = await checkPermissions();
    if (perm === 'prompt') {
      perm = await requestPermissions();
      // Let the permission activity finish before CameraX binds to ours.
      await new Promise((r) => setTimeout(r, 400));
    }
    if (perm !== 'granted') {
      denied = true;
      return;
    }
    // windowed: camera renders behind the (now transparent) webview.
    document.documentElement.classList.add('scanning');
    active = true;
    try {
      const result = await scan({ windowed: true, formats: [Format.QRCode] });
      active = false;
      if (!result.content.startsWith('otpauth://')) {
        error = $t('totp_scan_not_otpauth');
        return;
      }
      goto(`/totp/add?uri=${encodeURIComponent(result.content)}`, { replaceState: true });
    } catch (e) {
      active = false;
      fail(e);
    } finally {
      document.documentElement.classList.remove('scanning');
    }
  }

  /** The page says what to do in the UI's language; the plugin's own text goes to the log. */
  function fail(e: unknown) {
    console.error('QR scan failed:', formatError(e));
    error = $t('totp_scan_unavailable');
  }

  onMount(() => {
    start().catch(fail);
  });
  onDestroy(() => {
    document.documentElement.classList.remove('scanning');
    if (active) cancel().catch(() => {});
  });
</script>

<!-- The header of every phone page (.m-header); over the camera picture it is transparent. -->
<div class="m-page scan">
  <div class="m-header">
    <a class="m-ibtn" href="/totp/add" aria-label={$t('common_back')}>
      <Icon name="chevron-left" size={24} />
    </a>
    <h1 class="m-title">{$t('totp_scan_title')}</h1>
  </div>

  <div class="body">
  {#if denied}
    <div class="msg">
      <Icon name="alert-triangle" size={32} />
      <p>{$t('totp_scan_denied')}</p>
      <button class="btn btn-ghost" onclick={() => openAppSettings()}>{$t('totp_scan_open_settings')}</button>
    </div>
  {:else if error}
    <div class="msg">
      <Icon name="alert-triangle" size={32} />
      <p>{error}</p>
      <button class="btn btn-primary" onclick={() => { error = ''; start(); }}>{$t('totp_scan_retry')}</button>
    </div>
  {:else}
    <div class="frame"></div>
    <p class="hint">{$t('totp_scan_hint')}</p>
  {/if}
  </div>
</div>

<style>
  .scan { height: 100%; }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: var(--sp-4);
    background: var(--bg);
  }
  /* White only over the camera picture (while scanning); the page colours otherwise. */
  :global(html.scanning) .scan .m-header,
  :global(html.scanning) .scan .body { background: transparent; }
  :global(html.scanning) .scan .m-title,
  :global(html.scanning) .scan .m-ibtn,
  :global(html.scanning) .hint { color: #fff; }

  .frame {
    width: min(70vw, 300px);
    aspect-ratio: 1;
    margin-top: 16vh;
    border: 3px solid var(--accent);
    border-radius: 24px;
  }
  .hint {
    margin-top: var(--sp-5);
    color: var(--text-2);
    font-size: var(--fs-sm);
    text-align: center;
  }

  .msg {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-3);
    margin-top: 20vh;
    color: var(--text-2);
    text-align: center;
  }
  .msg p { margin: 0; }
</style>

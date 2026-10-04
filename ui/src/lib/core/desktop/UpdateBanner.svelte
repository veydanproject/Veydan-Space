<!--
  SPDX-FileCopyrightText: 2026 Veydan Project
  SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
-->
<script lang="ts">
  import { updaterStore } from '$lib/core/store/updater.svelte';
  import { api } from '$lib/core/api';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { product } from '$lib/core/product';

  // The product's own releases (products.json `repo`).
  const RELEASES_URL = `${product.repo}/releases/latest`;

  const visible = $derived(
    !updaterStore.bannerDismissed &&
      (updaterStore.status === 'available' ||
        updaterStore.status === 'downloading' ||
        updaterStore.status === 'installing')
  );
</script>

{#if visible}
  <div class="update-banner" role="status">
    <Icon name="download" size={14} />
    {#if updaterStore.status === 'downloading'}
      <span class="banner-text">{$t('settings_update_downloading')} {updaterStore.progress}%</span>
    {:else if updaterStore.status === 'installing'}
      <span class="banner-text">{$t('settings_update_installing')}</span>
    {:else}
      <span class="banner-text">{$t('update_banner_available', { version: updaterStore.version })}</span>
      {#if updaterStore.supported}
        <button class="banner-btn primary" onclick={() => updaterStore.install()}>
          {$t('update_banner_install')}
        </button>
      {:else}
        <button class="banner-btn primary" onclick={() => api.system.openUrl(RELEASES_URL)}>
          {$t('settings_update_open_releases')}
        </button>
      {/if}
      <button class="banner-btn" onclick={() => updaterStore.dismiss()}>
        {$t('update_banner_later')}
      </button>
    {/if}
    {#if updaterStore.status === 'available'}
      <button class="banner-close" onclick={() => updaterStore.dismiss()} title={$t('update_banner_later')}>
        <Icon name="x" size={13} />
      </button>
    {/if}
  </div>
{/if}

<style>
  .update-banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 16px;
    background: var(--accent-bg);
    border-bottom: 1px solid var(--accent-border);
    color: var(--accent-text);
    font-size: var(--fs-sm, 12.5px);
    flex-shrink: 0;
  }

  .banner-text {
    flex: 0 1 auto;
  }

  .banner-btn {
    appearance: none;
    border: 1px solid var(--accent-tint-border);
    background: transparent;
    color: var(--accent-text);
    border-radius: var(--radius-sm, 6px);
    padding: 2px 10px;
    font-size: inherit;
    font-family: inherit;
    cursor: pointer;
    transition: background 0.12s ease, border-color 0.12s ease;
  }

  .banner-btn:hover {
    background: var(--accent-tint);
    border-color: var(--accent-border);
  }

  .banner-btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }

  .banner-btn.primary:hover {
    background: var(--accent-hover);
    border-color: var(--accent-hover);
  }

  .banner-close {
    appearance: none;
    border: none;
    background: transparent;
    color: var(--accent-text);
    cursor: pointer;
    margin-left: auto;
    padding: 2px;
    display: flex;
    align-items: center;
    opacity: 0.7;
  }

  .banner-close:hover {
    opacity: 1;
  }
</style>

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
    <Icon name="circle-arrow-up" size={14} />
    {#if updaterStore.status === 'downloading'}
      <span class="banner-text">{$t('settings_update_downloading')} {updaterStore.progress}%</span>
    {:else if updaterStore.status === 'installing'}
      <span class="banner-text">{$t('settings_update_installing')}</span>
    {:else}
      <span class="banner-text">{$t('update_banner_available', { version: updaterStore.version })}</span>
      {#if updaterStore.supported}
        <button class="btn btn-sm btn-primary" onclick={() => updaterStore.install()}>
          {$t('update_banner_install')}
        </button>
      {:else}
        <button class="btn btn-sm btn-primary" onclick={() => api.system.openUrl(RELEASES_URL)}>
          {$t('settings_update_open_releases')}
        </button>
      {/if}
      <button class="btn btn-sm btn-ghost" onclick={() => updaterStore.dismiss()}>
        {$t('common_later')}
      </button>
    {/if}
    {#if updaterStore.status === 'available'}
      <button class="banner-close" onclick={() => updaterStore.dismiss()} title={$t('common_later')}>
        <Icon name="x" size={13} />
      </button>
    {/if}
  </div>
{/if}

<style>
  .update-banner {
    display: flex;
    align-items: center;
    gap: var(--sp-2-5);
    padding: var(--sp-1-5) var(--sp-4);
    background: var(--accent-bg);
    border-bottom: 1px solid var(--accent-border);
    color: var(--accent-text);
    font-size: var(--fs-sm);
    flex-shrink: 0;
  }

  .banner-text {
    flex: 0 1 auto;
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

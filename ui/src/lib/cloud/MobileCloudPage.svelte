<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The page «Account» on a phone (/cloud). Back leads to the settings, where the page opens from. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { cloudPage } from '$lib/cloud/status.svelte';
  import CloudBody from '$lib/cloud/CloudBody.svelte';
</script>

<div class="m-page">
  <div class="m-header">
    <a class="m-ibtn" href="/settings" aria-label={$t('common_back')}>
      <Icon name="chevron-left" size={24} />
    </a>
    <h1 class="m-title">{$t('cloud_account_title')}</h1>
    <span class="spacer"></span>
    {#if cloudPage.status?.account}
      <button class="m-ibtn" type="button" disabled={cloudPage.busy} onclick={() => cloudPage.refresh()} aria-label={$t('cloud_refresh')}>
        <span class:spin={cloudPage.busy}><Icon name="refresh-cw" size={20} /></span>
      </button>
    {/if}
  </div>

  <div class="m-body">
    <div class="lead">
      <CloudBody />
    </div>
  </div>
</div>

<style>
  /* The first card keeps a line's distance from the header. */
  .lead { margin-top: var(--sp-2); padding-bottom: var(--sp-6); }
  .m-ibtn span { display: inline-flex; }
</style>

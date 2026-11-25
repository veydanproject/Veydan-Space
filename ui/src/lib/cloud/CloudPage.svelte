<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The page «Account» on a computer (/cloud), opened from the account's card of the settings. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { cloudPage } from '$lib/cloud/status.svelte';
  import CloudBody from '$lib/cloud/CloudBody.svelte';
</script>

<div class="page cloud-page">
  <div class="page-header">
    <h1>{$t('cloud_account_title')}</h1>
    <span class="spacer"></span>
    {#if cloudPage.status?.account}
      <button class="icon-btn" type="button" disabled={cloudPage.busy} onclick={() => cloudPage.refresh()} aria-label={$t('common_refresh')} title={$t('common_refresh')}>
        <span class:spin={cloudPage.busy}><Icon name="refresh-cw" size={18} /></span>
      </button>
    {/if}
  </div>
  <CloudBody />
</div>

<style>
  .cloud-page { --page-max: none; padding-bottom: var(--sp-8); }
  /* The title on the line of the other pages. */
  .cloud-page > .page-header { align-items: flex-start; }
  .icon-btn span { display: inline-flex; }
  /* The mark 18 in the button 32 ends on the island's right edge: the button's 7 of air reach into the page's padding. */
  .cloud-page > .page-header > .icon-btn { margin-inline-end: -7px; }
</style>

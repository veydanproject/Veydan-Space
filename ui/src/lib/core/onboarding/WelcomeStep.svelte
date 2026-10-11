<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The first screen of the first start: the product by its name, and the two ways on — a new start, or the account the person has already (where the product has accounts). -->
<script lang="ts">
  import { locale, t, taglineOf } from '$lib/core/i18n';
  import { product } from '$lib/core/product';
  import { onboarding } from './store.svelte';

  let busy = $state(false);

  async function go(run: () => Promise<void>) {
    if (busy) return;
    busy = true;
    try {
      await run();
    } finally {
      busy = false;
    }
  }
</script>

<div class="ob-main center">
  <img class="ob-logo" src="/logo.png" alt="" />
  <h1 class="ob-title">{product.name}</h1>
  <p class="ob-text">{taglineOf($locale)}<br />{$t('ob_welcome_text')}</p>
</div>
<div class="ob-foot">
  <button type="button" class="btn btn-primary" disabled={busy} onclick={() => go(() => onboarding.next())}>{$t('ob_start')}</button>
  {#if onboarding.has('account')}
    <button type="button" class="btn btn-ghost" disabled={busy} onclick={() => go(() => onboarding.go('account', 'have'))}>{$t('ob_have_account')}</button>
  {/if}
</div>

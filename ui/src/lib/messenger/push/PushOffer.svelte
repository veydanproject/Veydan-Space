<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Asked once, on a phone, of a user who has not been asked. Until the answer
  is "yes" nothing is said to the push service or to the push server.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import OfferCard from '../shared/OfferCard.svelte';
  import { pushStore } from './pushStore.svelte';
  import { pushPrivacyKey } from './wording';
</script>

{#if pushStore.toOffer}
  <OfferCard icon="bell" title={$t('msg_push_offer_title')}>
    <p>{$t('msg_push_offer_text')}</p>
    {#snippet actions()}
      <button class="btn btn-primary" disabled={pushStore.busy} onclick={() => pushStore.setEnabled(true)}>
        {$t('msg_push_enable')}
      </button>
      <button class="btn btn-ghost" disabled={pushStore.busy} onclick={() => pushStore.decline()}>
        {$t('common_later')}
      </button>
    {/snippet}
    {#snippet note()}{$t(pushPrivacyKey())}{/snippet}
  </OfferCard>
{/if}

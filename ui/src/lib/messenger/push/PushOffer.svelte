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

  /** What it costs, told when asked: until then a «Подробнее» in its place, and the banner no taller than the bridge's. */
  let open = $state(false);
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
    {#snippet note()}
      {#if open}{$t(pushPrivacyKey())}{:else}<button type="button" class="more" onclick={() => (open = true)}>{$t('chan_keeper_more')}</button>{/if}
    {/snippet}
  </OfferCard>
{/if}

<style>
  /* A line of text, its press 44 tall, the line it shows no taller. */
  .more {
    display: inline-flex; align-items: center; min-height: 44px; margin-block: -12px; padding: 0;
    border: none; background: none; cursor: pointer;
    font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--accent-text);
  }
</style>

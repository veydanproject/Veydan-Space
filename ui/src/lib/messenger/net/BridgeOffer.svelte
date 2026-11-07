<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Shown when the direct way to the servers stopped carrying and a bridge
  carries: the app found that out by trying both. The user decides.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Notice from '$lib/core/ui/Notice.svelte';
  import OfferCard from '../shared/OfferCard.svelte';
  import { netErrorText } from './errors';
  import { netStore } from './netStore.svelte';

  const error = $derived(netErrorText(netStore.error, $t));
</script>

{#if netStore.toOffer}
  <OfferCard icon="building-bridge" title={$t('msg_bridge_offer_title')}>
    <p>{$t('msg_bridge_offer_text')}</p>
    {#if error}<Notice tone="danger">{error}</Notice>{/if}
    {#snippet actions()}
      <button class="btn btn-primary" disabled={netStore.busy} onclick={() => netStore.setMode('on')}>
        {$t('msg_bridge_enable')}
      </button>
      <button class="btn btn-ghost" disabled={netStore.busy} onclick={() => netStore.decline()}>
        {$t('common_later')}
      </button>
    {/snippet}
    {#snippet note()}{$t('msg_bridge_privacy')}{/snippet}
  </OfferCard>
{/if}

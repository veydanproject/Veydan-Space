<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Shown when the direct way to the servers stopped carrying and a bridge
  carries: the app found that out by trying both. The user decides.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { netErrorText } from './errors';
  import { netStore } from './netStore.svelte';

  const error = $derived(netErrorText(netStore.error, $t));
</script>

{#if netStore.toOffer}
  <div class="offer">
    <div class="head"><Icon name="shield" size={18} /> <strong>{$t('msg_bridge_offer_title')}</strong></div>
    <p>{$t('msg_bridge_offer_text')}</p>
    <p class="small">{$t('msg_bridge_privacy')}</p>
    {#if error}<p class="small error">{error}</p>{/if}
    <div class="actions">
      <button class="btn btn-primary btn-sm" disabled={netStore.busy} onclick={() => netStore.setMode('on')}>
        {$t('msg_bridge_enable')}
      </button>
      <button class="btn btn-ghost btn-sm" disabled={netStore.busy} onclick={() => netStore.decline()}>
        {$t('msg_bridge_offer_later')}
      </button>
    </div>
  </div>
{/if}

<style>
  .offer {
    margin: var(--sp-2) var(--sp-3); padding: var(--sp-3); flex-shrink: 0;
    display: flex; flex-direction: column; gap: var(--sp-2);
    border: 1px solid var(--accent-border); border-radius: var(--radius-sm); background: var(--accent-tint);
  }
  .head { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); }
  p { margin: 0; font-size: var(--fs-sm); color: var(--text-body); line-height: 1.45; }
  .small { font-size: var(--fs-xs); color: var(--text-2); }
  .small.error { color: var(--danger-text); }
  .actions { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
</style>

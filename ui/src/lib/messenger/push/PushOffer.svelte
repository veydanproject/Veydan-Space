<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Asked once, on a phone, of a user who has not been asked. Until the answer
  is "yes" nothing is said to the push service or to the push server.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { pushStore } from './pushStore.svelte';
</script>

{#if pushStore.toOffer}
  <div class="offer">
    <div class="head"><Icon name="bell" size={18} /> <strong>{$t('msg_push_offer_title')}</strong></div>
    <p>{$t('msg_push_offer_text')}</p>
    <p class="small">{$t('msg_push_privacy')}</p>
    <div class="actions">
      <button class="btn btn-primary btn-sm" disabled={pushStore.busy} onclick={() => pushStore.setEnabled(true)}>
        {$t('msg_push_enable')}
      </button>
      <button class="btn btn-ghost btn-sm" disabled={pushStore.busy} onclick={() => pushStore.decline()}>
        {$t('msg_push_offer_later')}
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
  .actions { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
</style>

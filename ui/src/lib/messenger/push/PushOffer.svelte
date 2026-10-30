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
    <div class="head"><Icon name="bell" size={16} /> <strong>{$t('msg_push_offer_title')}</strong></div>
    <p>{$t('msg_push_offer_text')}</p>
    <div class="actions">
      <button class="btn btn-primary" disabled={pushStore.busy} onclick={() => pushStore.setEnabled(true)}>
        {$t('msg_push_enable')}
      </button>
      <button class="btn btn-ghost" disabled={pushStore.busy} onclick={() => pushStore.decline()}>
        {$t('msg_push_offer_later')}
      </button>
    </div>
    <p class="small">{$t('msg_push_privacy')}</p>
  </div>
{/if}

<style>
  /* A compact card of the accent over the list: the title, the phrase and the buttons (the verb first) on top,
     what it costs smaller under them. */
  .offer {
    margin: var(--sp-2) var(--sp-3); padding: var(--sp-3); flex-shrink: 0;
    display: flex; flex-direction: column; gap: var(--sp-2);
    border: 1px solid var(--accent-border); border-radius: var(--radius-lg); background: var(--accent-bg);
  }
  .head { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-body); }
  .head strong { font-weight: var(--fw-bold); }
  p { margin: 0; font-size: var(--fs-base); color: var(--text-body); line-height: 1.45; }
  .small { font-size: var(--fs-2xs); color: var(--text-3); }
  /* Side by side, the verb the wider; a long word wraps them into two lines rather than clipping. */
  .actions { display: flex; flex-wrap: wrap; gap: var(--sp-2); }
  .actions .btn { flex: 1 1 auto; min-height: var(--control-h); justify-content: center; font-size: var(--fs-sm); }
  .actions .btn-primary { flex-grow: 1.4; }
</style>

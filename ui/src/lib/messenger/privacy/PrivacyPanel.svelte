<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { privacyStore } from './privacyStore.svelte';

  onMount(() => {
    privacyStore.load();
  });

  const view = $derived(privacyStore.view);
</script>

<div class="card privacy">
  <div class="card-title"><Icon name="eye-off" size={16} /> {$t('msg_privacy_title')}</div>

  {#if !view}
    <p class="muted">{privacyStore.error || $t('loading')}</p>
  {:else}
    <div class="block">
      <div class="line">
        <span>{$t('msg_privacy_read_receipts')}</span>
        <button
          class="toggle"
          class:on={view.read_receipts}
          disabled={privacyStore.busy}
          onclick={() => privacyStore.setReadReceipts(!view.read_receipts)}
          aria-pressed={view.read_receipts}
          aria-label={$t('msg_privacy_read_receipts')}
        ></button>
      </div>
      <p class="muted small">{$t('msg_privacy_read_receipts_hint')}</p>
    </div>
    <div class="block">
      <div class="line">
        <span>{$t('msg_privacy_presence')}</span>
        <button
          class="toggle"
          class:on={view.presence}
          disabled={privacyStore.busy}
          onclick={() => privacyStore.setPresence(!view.presence)}
          aria-pressed={view.presence}
          aria-label={$t('msg_privacy_presence')}
        ></button>
      </div>
      <p class="muted small">{$t('msg_privacy_presence_hint')}</p>
    </div>
    {#if privacyStore.error}<p class="warn">{privacyStore.error}</p>{/if}
  {/if}
</div>

<style>
  .privacy { display: flex; flex-direction: column; gap: var(--sp-3); }
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .muted { color: var(--text-2); font-size: var(--fs-sm); margin: 0; }
  .small { font-size: var(--fs-xs); }
  .warn { margin: 0; font-size: var(--fs-sm); color: var(--danger-text); }
  .line { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); font-size: var(--fs-sm); }
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
</style>

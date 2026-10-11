<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The cards of the vcloud bot about recovery, as a banner over any page
  (cloud-recovery.md §8.5 step 4, §10): «Кто-то запросил восстановление…»
  with «Это не я — отменить», «Выдана копия для восстановления…» with the
  way to the recovery, «Доступ восстановлен с нового устройства». The
  alerts come from Rust (`cloud://recovery-alerts`) and are dismissed there.
-->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { t, locale } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { dateOf } from '$lib/cloud/account';
  import { cardsOnScreen } from '$lib/cloud/onScreen.svelte';
  import { alertsStore, socialErrorKey } from '$lib/cloud/social.svelte';

  let stop: (() => void) | null = null;
  onMount(() => {
    void alertsStore.load();
    void alertsStore.listen().then((s) => (stop = s));
  });
  onDestroy(() => stop?.());

  /** «Открыть восстановление»: the part of the page of the cloud; on that page already, it is scrolled to. */
  function openRecovery(id: string) {
    void alertsStore.dismiss(id);
    const there = page.url.pathname === '/cloud' ? document.getElementById('recovery-key') : null;
    if (there) there.scrollIntoView({ block: 'center', behavior: 'smooth' });
    else void goto('/cloud?section=recovery-key');
  }

  /** The alerts to show: one whose card is open in the chat of the bot right now is said there. */
  const alerts = $derived(alertsStore.alerts.filter((a) => !cardsOnScreen.has(a.id, a.request_id)));

  const errorText = $derived.by(() => {
    const key = socialErrorKey(alertsStore.error);
    return key ? $t(key) : (alertsStore.error?.message ?? '');
  });
</script>

{#if alertsStore.cancelled?.frozen_until}
  <div class="banner ok" role="status">
    <span class="mark"><Icon name="check" size={16} /></span>
    <div class="body">
      <p class="text">{$t('cloud_alert_cancelled', { date: dateOf(alertsStore.cancelled.frozen_until, $locale) })}</p>
      <div class="buttons">
        <button class="btn btn-ghost btn-sm" type="button" onclick={() => (alertsStore.cancelled = null)}>{$t('cloud_alert_close')}</button>
      </div>
    </div>
  </div>
{/if}
{#each alerts as card (card.id)}
  <div class="banner" class:danger={card.t === 'recovery.requested'} class:warn={card.t === 'recovery.escrow_read'} class:ok={card.t === 'recovery.restored'} role="alert">
    <span class="mark"><Icon name={card.t === 'recovery.restored' ? 'check' : 'alert-triangle'} size={16} /></span>
    <div class="body">
      <p class="text">
        {#if card.t === 'recovery.requested'}{$t('cloud_alert_requested')}
        {:else if card.t === 'recovery.escrow_read'}{$t('cloud_alert_escrow_read')}
        {:else}{$t('cloud_alert_restored')}{/if}
      </p>
      <span class="when">{dateOf(card.at, $locale)}</span>
      <div class="buttons">
        {#if card.t === 'recovery.requested' && card.request_id}
          <button class="btn btn-danger btn-sm" type="button" disabled={alertsStore.busy} onclick={() => alertsStore.cancel(card.request_id ?? '')}>
            {$t('cloud_alert_not_me')}
          </button>
        {:else if card.t === 'recovery.escrow_read'}
          <button class="btn btn-primary btn-sm" type="button" onclick={() => openRecovery(card.id)}>
            {$t('cloud_alert_open_recovery')}
          </button>
        {/if}
        <button class="btn btn-ghost btn-sm" type="button" onclick={() => alertsStore.dismiss(card.id)}>{$t('cloud_alert_close')}</button>
      </div>
    </div>
  </div>
{/each}
{#if errorText}<div class="banner danger" role="alert"><span class="mark"><Icon name="alert-triangle" size={16} /></span><div class="body"><p class="text">{errorText}</p></div></div>{/if}

<style>
  /* A card of the column of Banners.svelte: the tone on the left edge and the mark, the surface the app's own. */
  .banner {
    display: flex; align-items: flex-start; gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4); border-radius: var(--radius-md);
    background: var(--surface-drawer); border: 1px solid var(--border-2); box-shadow: var(--shadow-lg);
    color: var(--text); font-size: var(--fs-sm);
  }
  .mark {
    display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; border-radius: 50%; flex-shrink: 0;
    background: var(--accent-tint); color: var(--accent-text);
  }
  .danger { border-color: var(--danger-border); }
  .danger .mark { background: var(--danger-bg); color: var(--danger-text); }
  .warn { border-color: var(--warn-border); }
  .warn .mark { background: var(--warn-bg); color: var(--warn-text); }
  .ok .mark { background: var(--success-bg); color: var(--success-text); }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .text { margin: 0; line-height: 1.45; }
  .when { font-size: var(--fs-xs); color: var(--text-3); margin-top: calc(-1 * var(--sp-1)); }
  .buttons { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
</style>

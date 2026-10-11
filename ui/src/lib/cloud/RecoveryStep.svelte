<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The Recovery Key as the last step of the first start (cloud-recovery.md
  §6): the 12 words are shown here, in the wizard, with the check of three
  of them (RecoveryKeyCard) — not on the page «Account» the person would
  be sent to and back from. The same rules as there: the copy for recovery
  carries the vault of sync, so a device that syncs nowhere gets the vault
  of Veydan Cloud first, and a storage of the person's own that is not
  connected yet is finished in the settings of sync; a lock that is on asks
  its secret. An account still on its way (the free plan is activated in
  the background after the servers are chosen) is waited for. The step is
  passed by, and is not there at all for an identity that cannot have an
  account (none made, or the person's own servers): the wizard goes on.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t, locale } from '$lib/core/i18n';
  import { joinErrorKey } from '$lib/core/join-error';
  import { appLock } from '$lib/core/lock/store.svelte';
  import RecoveryKeyCard from '$lib/core/lock/RecoveryKeyCard.svelte';
  import type { OnboardingStepProps } from '$lib/core/module';
  import { SYNC_HREF } from '$lib/core/onboarding/setup';
  import { asCloudError, cloudApi, type CloudRecoveryKeyStatus } from '$lib/cloud/api';
  import { dateOf } from '$lib/cloud/account';
  import { ensureAccount } from '$lib/cloud/ensure';
  import { recoveryRefusal } from '$lib/cloud/recoveryKey';
  import { connectCloudVault, loadVaultState, type VaultState } from '$lib/cloud/vault';
  import Notice from '$lib/cloud/Notice.svelte';

  let { next, detour }: OnboardingStepProps = $props();

  let ready = $state(false);
  /** The free plan is still being activated: the step waits for the account. */
  let activating = $state(false);
  let status = $state<CloudRecoveryKeyStatus | null>(null);
  let plan = $state<string | null>(null);
  /** The vault of sync of this device, as the copy for recovery needs it. */
  let vault = $state<VaultState>('joined');
  let secret = $state('');
  let busy = $state(false);
  /** The storage does not know a new account yet: the step waits for it and says so. */
  let preparing = $state(false);
  let error = $state('');
  let words = $state<string | null>(null);

  const lockOn = $derived(appLock.status.enabled);
  const issued = $derived(status?.issued === true);

  onMount(async () => {
    try {
      let cloud = await cloudApi.status();
      if (!cloud.account) {
        // No account can be here (none made, or the person's own servers): nothing to recover, the wizard goes on.
        if ((cloud.servers ?? (cloud.configured ? 'veydan' : 'none')) !== 'veydan') return next();
        // It is on its way: the proof of work of the free plan takes seconds.
        activating = true;
        await ensureAccount();
        cloud = await cloudApi.status();
        activating = false;
        if (!cloud.account) return next();
      }
      plan = cloud.account.plan;
      [status, vault] = await Promise.all([cloudApi.recoveryStatus(), loadVaultState()]);
      await appLock.refresh();
      ready = true;
    } catch {
      // The cloud does not answer now: the words wait on the page «Account».
      next();
    }
  });

  async function issue() {
    if (busy) return;
    busy = true;
    error = '';
    try {
      if (vault === 'none') {
        await connectCloudVault(plan, { onWait: () => (preparing = true) });
        vault = 'joined';
        preparing = false;
      }
      words = (await cloudApi.recoveryKeyShowOnce(lockOn ? secret : null)).words;
      secret = '';
    } catch (e) {
      // The vault of the account was not joined: the words of the settings of sync.
      const joinKey = joinErrorKey(e);
      error = joinKey ? $t(joinKey) : recoveryRefusal(asCloudError(e), (key) => $t(key));
    } finally {
      busy = false;
      preparing = false;
    }
  }
</script>

{#if !ready}
  <div class="ob-main center" role="status">
    <span class="spinner" aria-hidden="true"></span>
    {#if activating}<p class="ob-text activating">{$t('cloud_activating')}</p>{/if}
  </div>
{:else if words}
  <div class="ob-main">
    <RecoveryKeyCard code={words} ondone={next} />
  </div>
{:else if issued}
  <div class="ob-main">
    <span class="ob-tile"><Icon name="check-circle" size={28} /></span>
    <h1 class="ob-title">{$t('cloud_recovery_key')}</h1>
    <p class="ob-text">
      {status?.escrow_written_at ? $t('cloud_recovery_issued', { date: dateOf(status.escrow_written_at, $locale) }) : $t('cloud_recovery_already')}
    </p>
  </div>
  <div class="ob-foot">
    <button type="button" class="btn btn-primary" onclick={next}>{$t('ob_done')}</button>
  </div>
{:else}
  <form class="ob-main" id="cloud-ob-recovery" onsubmit={(e) => { e.preventDefault(); void issue(); }}>
    <span class="ob-tile"><Icon name="key" size={28} /></span>
    <h1 class="ob-title">{$t('cloud_setup_words')}</h1>
    <p class="ob-text">{$t('cloud_recovery_how_text')}</p>
    {#if vault === 'none'}
      <Notice tone="info">{$t('cloud_recovery_turns_sync_on')}</Notice>
    {:else if vault === 'own'}
      <Notice tone="warn">{$t('cloud_recovery_no_vault')}</Notice>
    {/if}
    {#if lockOn && vault !== 'own'}
      <label class="field">
        <span>{$t('cloud_recovery_lock_secret')}</span>
        <input type="password" bind:value={secret} autocomplete="current-password" disabled={busy} />
      </label>
    {/if}
    {#if preparing}<p class="ob-wait wait" role="status"><span class="spinner" aria-hidden="true"></span>{$t('ob_sync_preparing')}</p>{/if}
    {#if error}<p class="ob-error" role="alert">{error}</p>{/if}
  </form>
  <div class="ob-foot">
    {#if vault === 'own'}
      <button type="button" class="btn btn-primary" onclick={() => detour(SYNC_HREF)}>{$t('ob_sync_setup')}</button>
    {:else}
      <button type="submit" form="cloud-ob-recovery" class="btn btn-primary" disabled={busy || (lockOn && !secret)}>{$t('cloud_recovery_show_words')}</button>
    {/if}
    <button type="button" class="ob-skip" disabled={busy} onclick={next}>{$t('ob_later')}</button>
  </div>
{/if}

<style>
  .field { display: flex; flex-direction: column; gap: 6px; margin-top: var(--sp-4); font-size: var(--fs-sm); }
  .field span { color: var(--text-3); font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.5px; }
  .field input {
    font: inherit; color: var(--text); min-height: 44px;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm);
    padding: 8px 10px;
  }
  .field input:focus { outline: none; border-color: var(--accent-border); }
  .wait { margin: var(--sp-4) 0 0; }
  .activating { margin: var(--sp-4) 0 0; }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Устройства и доступ» on the page of the cloud (cloud-recovery.md §6, §8,
  §11): one list of the ways into the account — another device, the
  Recovery Key, the trusted people — and the help to a friend. The Recovery
  Key is set up and reissued here: both ask the secret of the lock and show
  the 12 words once, with the check of three words (RecoveryKeyCard).
  Another device and the help to a friend go over the relays and need no
  account; the Recovery Key and the trusted people keep their copy in the
  account, so without one they are not offered.
  The copy for recovery carries the vault of sync (§4): on a device that
  syncs nowhere the setup turns on Veydan Cloud first and says so; a
  storage of the user's own that is chosen but not connected is finished
  in the settings of sync, and the way there is shown.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t, locale } from '$lib/core/i18n';
  import { joinErrorKey } from '$lib/core/join-error';
  import { isMobile } from '$lib/core/platform';
  import { appLock } from '$lib/core/lock/store.svelte';
  import RecoveryKeyCard from '$lib/core/lock/RecoveryKeyCard.svelte';
  import { asCloudError, cloudApi, type CloudError, type CloudRecoveryInfo, type CloudRecoveryKeyStatus } from '$lib/cloud/api';
  import { dateOf } from '$lib/cloud/account';
  import { recoveryRefusal } from '$lib/cloud/recoveryKey';
  import { cloudPage } from '$lib/cloud/status.svelte';
  import { connectCloudVault, loadVaultState, type VaultState } from '$lib/cloud/vault';
  import Actions from '$lib/cloud/Actions.svelte';
  import NavRow from '$lib/cloud/NavRow.svelte';
  import Notice from '$lib/cloud/Notice.svelte';
  import RowList from '$lib/cloud/RowList.svelte';

  let {
    hasAccount,
    recovery = null,
  }: {
    hasAccount: boolean;
    /** What the server knows of the trusted people of the account. */
    recovery?: CloudRecoveryInfo | null;
  } = $props();

  let status = $state<CloudRecoveryKeyStatus | null>(null);
  let loadError = $state<CloudError | null>(null);
  let secret = $state('');
  let asking = $state<'setup' | 'reissue' | null>(null);
  let busy = $state(false);
  let error = $state('');
  let words = $state<string | null>(null);
  let how = $state(false);
  /** The last refusal was `no_sync_vault`: the way to the settings of sync is shown. */
  let noVault = $state(false);
  /** The vault of sync of this device, asked when the setup opens. */
  let vault = $state<VaultState>('joined');
  /** The settings of sync: a card of the settings on a computer, a page on a phone. */
  const syncHref = isMobile ? '/settings/sync' : '/settings#sync';

  const lockOn = $derived(appLock.status.enabled);
  const issued = $derived(status?.issued === true);
  const keyLine = $derived.by(() => {
    if (!status) return loadError ? loadError.message || loadError.code : '';
    if (!status.issued) return $t('cloud_recovery_not_issued');
    return status.escrow_written_at
      ? $t('cloud_recovery_issued', { date: dateOf(status.escrow_written_at, $locale) })
      : $t('cloud_recovery_key');
  });
  const trusteesLine = $derived.by(() => {
    if (!issued) return $t('cloud_trustees_need_key');
    if (recovery?.configured && recovery.k && recovery.n) return $t('cloud_recovery_on', { k: String(recovery.k), n: String(recovery.n) });
    return $t('cloud_trustees_off');
  });

  onMount(() => {
    void appLock.refresh();
    void load();
  });

  async function load() {
    try {
      status = await cloudApi.recoveryStatus();
      loadError = null;
    } catch (e) {
      loadError = asCloudError(e);
    }
  }

  async function start(what: 'setup' | 'reissue') {
    error = '';
    secret = '';
    noVault = false;
    if (what === 'setup') vault = await loadVaultState();
    asking = what;
  }

  async function issue() {
    if (!asking) return;
    busy = true;
    error = '';
    noVault = false;
    try {
      if (asking === 'setup' && vault === 'none') {
        await connectCloudVault(cloudPage.status?.account?.plan ?? cloudPage.status?.kept?.plan ?? null);
        vault = 'joined';
      }
      const current = lockOn ? secret : null;
      const issuedKey = asking === 'setup' ? await cloudApi.recoveryKeyShowOnce(current) : await cloudApi.recoveryKeyReissue(current);
      words = issuedKey.words;
      asking = null;
      secret = '';
      await load();
    } catch (e) {
      const refusal = asCloudError(e);
      noVault = refusal.code === 'no_sync_vault';
      // The vault of the account was not joined: the words of the settings of sync.
      const joinKey = joinErrorKey(e);
      error = joinKey ? $t(joinKey) : recoveryRefusal(refusal, (key) => $t(key));
    } finally {
      busy = false;
    }
  }

  function done() {
    words = null;
    void appLock.refresh();
  }
</script>

<RowList label={$t('cloud_access_title')} id="recovery-key">
  <NavRow icon="smartphone" title={$t('cloud_link_add_entry')} sub={$t('cloud_link_add_sub')} href="/cloud/add-device" />
  {#if hasAccount}
    <NavRow icon="key" title={$t('cloud_recovery_key')} sub={keyLine} tone={status && !issued ? 'warn' : undefined}>
      {#snippet trailing()}
        {#if !asking && !words && status}
          {#if issued}
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => start('reissue')}>{$t('cloud_recovery_reissue_short')}</button>
          {:else}
            <button class="btn btn-primary btn-sm" type="button" onclick={() => start('setup')}>{$t('cloud_recovery_setup_short')}</button>
          {/if}
        {/if}
      {/snippet}
    </NavRow>

    {#if words}
      <div class="panel" class:m={isMobile}>
        <RecoveryKeyCard code={words} ondone={done} />
      </div>
    {:else if asking}
      <form class="panel" class:m={isMobile} onsubmit={(e) => { e.preventDefault(); void issue(); }}>
        <h3>{asking === 'setup' ? $t('cloud_recovery_setup') : $t('cloud_recovery_reissue')}</h3>
        {#if asking === 'reissue'}
          <Notice tone="warn">{$t('cloud_recovery_reissue_hint')}</Notice>
        {:else}
          {#if lockOn && appLock.status.recovery_version !== null}
            <Notice tone="warn">{$t('cloud_recovery_setup_replaces')}</Notice>
          {:else}
            <p class="text">{$t('cloud_recovery_how_text')}</p>
          {/if}
          {#if vault === 'none'}
            <Notice tone="info">{$t('cloud_recovery_turns_sync_on')}</Notice>
          {:else if vault === 'own'}
            <Notice tone="warn">{$t('cloud_recovery_no_vault')}</Notice>
          {/if}
        {/if}
        {#if lockOn && !(asking === 'setup' && vault === 'own')}
          <div class="form-group">
            <label for="cloud-recovery-secret">{$t('cloud_recovery_lock_secret')}</label>
            <!-- svelte-ignore a11y_autofocus -->
            <input id="cloud-recovery-secret" type="password" bind:value={secret} autocomplete="current-password" disabled={busy} autofocus />
          </div>
        {/if}
        {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
        <Actions>
          <button class="btn btn-ghost" type="button" disabled={busy} onclick={() => (asking = null)}>{$t('common_cancel')}</button>
          {#if asking === 'setup' && vault === 'own'}
            <a class="btn btn-primary" href={syncHref}>{$t('nav_settings')} → {$t('settings_sync_section')}</a>
          {:else}
            <button class="btn btn-primary" type="submit" disabled={busy || (lockOn && !secret)}>
              {asking === 'setup' ? $t('cloud_recovery_show_words') : $t('cloud_recovery_reissue')}
            </button>
          {/if}
        </Actions>
        {#if noVault}
          <a class="link" href={syncHref}>{$t('nav_settings')} → {$t('settings_sync_section')}</a>
        {/if}
      </form>
    {/if}

    <NavRow icon="users" title={$t('cloud_trustees_entry')} sub={trusteesLine} href="/cloud/trustees" disabled={!issued} />
  {/if}
  <NavRow icon="shield" title={$t('cloud_trustees_help_entry')} sub={$t('cloud_help_sub')} href="/cloud/help" />
</RowList>

{#if hasAccount}
  <div class="how" class:m={isMobile}>
    <button class="link" type="button" aria-expanded={how} onclick={() => (how = !how)}>{$t('cloud_recovery_how')}</button>
    {#if how}<Notice tone="info">{$t('cloud_recovery_how_text')}</Notice>{/if}
  </div>
{/if}

<style>
  .panel { display: flex; flex-direction: column; gap: var(--sp-3); padding: var(--sp-5); border-bottom: 1px solid var(--border); background: var(--surface-2); }
  .panel.m { padding: var(--sp-4); }
  h3 { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); }
  .text { margin: 0; font-size: var(--fs-base); color: var(--text-2); line-height: 1.5; }
  .how { display: flex; flex-direction: column; align-items: flex-start; gap: var(--sp-2); margin-top: var(--sp-1); }
  .link {
    display: inline-flex; align-items: center; min-height: 32px; padding: 0 var(--sp-1); border: 0; background: none; cursor: pointer;
    color: var(--accent-text-3); font: inherit; font-size: var(--fs-sm); font-weight: var(--fw-semibold);
  }
  .how.m .link { min-height: 44px; font-size: 14px; }
</style>

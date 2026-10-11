<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The account at the top of the settings of a phone (`into: 'account'`): who
  it is and its plan — the way to the page «Account» — and under it how
  sync stands, the way to its settings. A device without an account gets the
  invitation instead. The plan is read from what the device keeps: opening
  the settings asks the server nothing.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { accountName, accountSource, loadAccount, type AccountProfile } from '$lib/core/account';
  import { api, onSyncStatus, type SyncStatus } from '$lib/core/mobile/api';
  import { t } from '$lib/core/mobile/i18n';
  import { cloudApi } from '$lib/cloud/api';
  import { planName } from '$lib/cloud/plans';

  let profile = $state<AccountProfile | null | undefined>(undefined);
  let plan = $state('');
  let sync = $state<SyncStatus | null>(null);

  const letter = $derived(profile ? ([...accountName(profile).trim()][0] ?? '?').toUpperCase() : '');
  const syncLabel = $derived(
    sync?.running ? $t('settings_sync_running') : sync?.enabled && sync.joined ? $t('settings_sync_connected') : $t('settings_sync_disconnected'),
  );

  onMount(() => {
    void loadAccount().then((p) => (profile = p));
    cloudApi
      .limits()
      .then((l) => (plan = l.plan ?? ''))
      .catch(() => {
        // No account kept here: the row leads to the page all the same.
      });
    const readSync = () =>
      api.sync
        .status()
        .then((s) => (sync = s))
        .catch(() => {});
    void readSync();
    const stop = onSyncStatus(() => void readSync());
    return () => void stop.then((f) => f());
  });
</script>

{#if profile}
  <a class="m-row who" href="/cloud">
    <span class="ava" aria-hidden="true">{letter}</span>
    <span class="words">
      <strong>{accountName(profile)}</strong>
      <span class="sub">{plan ? `${$t('cloud_settings_title')} · ${planName(null, plan)}` : $t('cloud_account_title')}</span>
    </span>
    <span class="chev"><Icon name="chevron-right" size={16} /></span>
  </a>
{:else if profile === null}
  <a class="m-row who" href={accountSource()?.createHref ?? '/cloud'}>
    <span class="ava ghost" aria-hidden="true"><Icon name="user" size={22} /></span>
    <span class="words">
      <strong>{$t('cloud_account_none')}</strong>
      <span class="sub">{$t('cloud_account_invite')}</span>
    </span>
    <span class="go">{$t('cloud_account_create')}</span>
  </a>
{/if}
<a class="m-row" href="/settings/sync">
  <span class="m-row-label">{$t('settings_sync_section')}</span>
  <span class="m-row-value" class:ok={sync?.enabled && sync?.joined}>{sync ? syncLabel : ''}</span>
  <span class="chev"><Icon name="chevron-right" size={16} /></span>
</a>

<style>
  .who { min-height: 72px; padding-top: var(--sp-3); padding-bottom: var(--sp-3); }
  .ava {
    display: inline-flex; align-items: center; justify-content: center; width: 48px; height: 48px; flex-shrink: 0; border-radius: 50%;
    background: var(--accent-grad); color: #fff; font-size: 20px; font-weight: 800;
  }
  .ava.ghost { background: var(--accent-tint); color: var(--accent); }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  strong { font-size: 16px; font-weight: 700; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: 13px; color: var(--text-2); line-height: 1.35; }
  .go { flex-shrink: 0; padding: 6px 12px; border-radius: 999px; background: var(--accent-tint); color: var(--accent-text); font-size: 13px; font-weight: 700; }
  .ok { color: var(--success-text); font-weight: 600; }
</style>

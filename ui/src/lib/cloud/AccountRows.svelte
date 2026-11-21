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
  import Avatar from '$lib/cloud/Avatar.svelte';
  import { accountName, accountSource, loadAccount, type AccountProfile } from '$lib/core/account';
  import { api, onSyncStatus, type SyncStatus } from '$lib/core/mobile/api';
  import { t } from '$lib/core/mobile/i18n';
  import { cloudApi } from '$lib/cloud/api';
  import { planName } from '$lib/cloud/plans';

  let profile = $state<AccountProfile | null | undefined>(undefined);
  let plan = $state('');
  let sync = $state<SyncStatus | null>(null);

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
  <a class="list-row who" href="/cloud">
    <Avatar name={accountName(profile)} size={48} />
    <span class="words">
      <strong>{accountName(profile)}</strong>
      <span class="sub">{plan ? `${$t('cloud_settings_title')} · ${planName(null, plan)}` : $t('cloud_account_title')}</span>
    </span>
    <span class="chev"><Icon name="chevron-right" size={16} /></span>
  </a>
{:else if profile === null}
  <a class="list-row who" href={accountSource()?.createHref ?? '/cloud'}>
    <Avatar size={48} ghost />
    <span class="words">
      <strong>{$t('cloud_account_none')}</strong>
      <span class="sub">{$t('cloud_account_invite')}</span>
    </span>
    <span class="go">{$t('common_create')}</span>
  </a>
{/if}
<a class="list-row dense sync" href="/settings/sync">
  <span class="tile"><Icon name="refresh-cw" size={20} /></span>
  <span class="words">
    <span class="list-row-label">{$t('settings_sync_section')}</span>
    <span class="sub" class:ok={sync?.enabled && sync?.joined}>{sync ? syncLabel : ''}</span>
  </span>
  <span class="chev"><Icon name="chevron-right" size={16} /></span>
</a>

<style>
  /* The rows of the design kit's settings: a tile, the name, its value under it. */
  .list-row { gap: var(--sp-3); padding-left: var(--sp-3); padding-right: var(--sp-3); }
  .tile {
    display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0;
    width: 36px; height: 36px; margin: 0 var(--sp-1-5); border-radius: var(--radius);
    background: var(--m-seg); color: var(--text-soft);
  }
  .sync .list-row-label { flex: none; font-weight: var(--fw-medium); }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  strong { font-size: var(--fs-md); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: var(--fs-sm); color: var(--text-2); line-height: var(--lh-snug); }
  .go { flex-shrink: 0; padding: var(--sp-1-5) var(--sp-3); border-radius: var(--radius-pill); background: var(--accent-tint); color: var(--accent-text); font-size: var(--fs-sm); font-weight: var(--fw-bold); }
  .ok { color: var(--success-text); font-weight: var(--fw-semibold); }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The account as the first card of the settings of a computer: who it is and its plan, and the way to the page «Account» (/cloud), where the plan, sync, the devices and the recovery are; a computer without an account gets the invitation. The plan is read from what the device keeps: opening the settings asks the server nothing. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { accountName, accountSource, loadAccount, shortNpub, type AccountProfile } from '$lib/core/account';
  import { t } from '$lib/core/i18n';
  import { cloudApi } from '$lib/cloud/api';
  import { planName } from '$lib/cloud/plans';

  let profile = $state<AccountProfile | null | undefined>(undefined);
  let plan = $state('');
  const letter = $derived(profile ? ([...accountName(profile).trim()][0] ?? '?').toUpperCase() : '');

  onMount(() => {
    void loadAccount().then((p) => (profile = p));
    cloudApi
      .limits()
      .then((l) => (plan = l.plan ?? ''))
      .catch(() => {
        // No account kept here: the card leads to the page all the same.
      });
  });
</script>

{#if profile}
  <div class="who">
    <span class="ava" aria-hidden="true">{letter}</span>
    <span class="words">
      <strong>{accountName(profile)}</strong>
      <span class="sub">{plan ? `${$t('cloud_settings_title')} · ${planName(null, plan)}` : shortNpub(profile.npub)}</span>
    </span>
    <a class="btn btn-ghost btn-sm" href="/cloud"><Icon name="user" size={14} />{$t('cloud_account_open')}</a>
  </div>
{:else if profile === null}
  <div class="who">
    <span class="ava ghost" aria-hidden="true"><Icon name="user" size={20} /></span>
    <span class="words">
      <strong>{$t('cloud_account_none')}</strong>
      <span class="sub">{$t('cloud_account_invite')}</span>
    </span>
    <a class="btn btn-primary btn-sm" href={accountSource()?.createHref ?? '/cloud'}>{$t('cloud_account_create')}</a>
  </div>
{/if}

<style>
  .who { display: flex; align-items: center; gap: var(--sp-3); }
  .ava {
    display: inline-flex; align-items: center; justify-content: center; width: 44px; height: 44px; flex-shrink: 0; border-radius: 50%;
    background: var(--accent-grad); color: #fff; font-size: var(--fs-lg); font-weight: 800;
  }
  .ava.ghost { background: var(--accent-tint); color: var(--accent-text); }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  strong { font-size: var(--fs-md); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: var(--fs-sm); color: var(--text-2); }
</style>

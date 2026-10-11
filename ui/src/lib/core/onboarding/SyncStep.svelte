<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Sync at the first start. With an account on the project's servers the
  step offers Veydan Cloud by one button (vcloud-plan.md 4.5): the free plan
  is activated where it is not yet, and the vault of the account is joined
  when another device made it already — then the step says so and offers to
  connect this device — or created. A storage of the person's own is set up
  in the settings of sync, where its form is: the wizard steps aside and
  comes back. Without an account, or on the person's own servers, the own
  storage is the only way, with a word of what an account would give.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/core/api';
  import { accountSource, loadAccount } from '$lib/core/account';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { joinErrorKey } from '$lib/core/join-error';
  import { isMobile } from '$lib/core/platform';
  import { cloudVaultExists, connectCloudVault, loadCloud, type CloudView, type VaultStage } from '$lib/core/sync/cloud';
  import { formatError } from '$lib/core/utils';
  import { SYNC_HREF } from './setup';
  import { onboarding } from './store.svelte';

  let ready = $state(false);
  let joined = $state(false);
  let cloud = $state<CloudView>({ choice: 'none', plan: null, usage: null });
  /** The account has a vault already: this device is connected to it, not the first to sync. */
  let found = $state(false);
  /** The product has accounts, and this device has none yet. */
  let noAccount = $state(false);
  let pick = $state<'cloud' | 'own'>('cloud');
  let busy = $state(false);
  /** Where the turning on is, shown as it goes: the account, its storage, this device. */
  let stage = $state<VaultStage>('account');
  const STAGES: { id: VaultStage; title: 'ob_sync_stage_account' | 'ob_sync_stage_storage' | 'ob_sync_stage_device' }[] = [
    { id: 'account', title: 'ob_sync_stage_account' },
    { id: 'storage', title: 'ob_sync_stage_storage' },
    { id: 'device', title: 'ob_sync_stage_device' },
  ];
  const at = $derived(STAGES.findIndex((x) => x.id === stage));
  let error = $state('');

  const offered = $derived(cloud.choice !== 'none');

  onMount(async () => {
    try {
      joined = (await api.sync.status()).joined;
    } catch {
      joined = false;
    }
    cloud = await loadCloud();
    if (!joined && cloud.choice === 'option') found = await cloudVaultExists();
    if (cloud.choice === 'none' && onboarding.has('account')) {
      noAccount = accountSource() !== null && (await loadAccount()) === null;
    }
    ready = true;
  });

  async function enable() {
    if (busy) return;
    busy = true;
    stage = 'account';
    error = '';
    try {
      await connectCloudVault(cloud.plan, { onStage: (now) => (stage = now) });
      await onboarding.next();
    } catch (e) {
      const key = joinErrorKey(e);
      error = key ? $t(key) : formatError(e);
    } finally {
      busy = false;
    }
  }

  const own = () => onboarding.detour(SYNC_HREF);
</script>

{#if !ready}
  <div class="ob-main center"><span class="spinner" aria-hidden="true"></span></div>
{:else if busy}
  <!-- The cloud is being turned on: a new account's storage takes up to a minute, and the way is shown, not a button that stands still. -->
  <div class="ob-main center" role="status" aria-live="polite">
    <div class="prep" aria-hidden="true">
      <span class="ring"></span>
      <span class="core"><Icon name="cloud" size={34} /></span>
    </div>
    <h1 class="ob-title">{$t('ob_sync_prep_title')}</h1>
    <p class="ob-text">{$t('ob_sync_preparing')}</p>
    <ol class="stages">
      {#each STAGES as x, i (x.id)}
        <li class:done={i < at} class:on={i === at}>
          <span class="mark">{#if i < at}<Icon name="check" size={13} />{:else if i === at}<span class="spinner"></span>{/if}</span>
          {$t(x.title)}
        </li>
      {/each}
    </ol>
  </div>
{:else if joined}
  <div class="ob-main">
    <span class="ob-tile"><Icon name="check-circle" size={28} /></span>
    <h1 class="ob-title">{$t('ob_sync_on')}</h1>
    <p class="ob-text">{$t('ob_sync_on_text')}</p>
  </div>
  <div class="ob-foot">
    <button type="button" class="btn btn-primary" onclick={() => onboarding.next()}>{$t('modules_first_continue')}</button>
  </div>
{:else if offered && found}
  <div class="ob-main">
    <span class="ob-tile"><Icon name="cloud" size={28} /></span>
    <h1 class="ob-title">{$t('ob_sync_found_title')}</h1>
    <p class="ob-text">{$t('ob_sync_found_text')}</p>
    {#if error}<p class="ob-error" role="alert">{error}</p>{/if}
  </div>
  <div class="ob-foot">
    <button type="button" class="btn btn-primary" disabled={busy} onclick={enable}>{$t('ob_sync_connect')}</button>
    <button type="button" class="ob-skip" disabled={busy} onclick={() => onboarding.next()}>{$t('ob_later')}</button>
  </div>
{:else if offered}
  <div class="ob-main">
    <span class="ob-tile"><Icon name="refresh-cw" size={28} /></span>
    <h1 class="ob-title">{$t('settings_sync_section')}</h1>
    <p class="ob-text">{$t('ob_sync_text')}</p>
    <div class="ob-opts" role="radiogroup">
      <button type="button" class="ob-opt" class:on={pick === 'cloud'} role="radio" aria-checked={pick === 'cloud'} disabled={busy} onclick={() => (pick = 'cloud')}>
        <span class="ob-opt-ic"><Icon name="cloud" size={20} /></span>
        <span class="ob-opt-b">
          <span class="ob-opt-t">{$t('settings_sync_backend_cloud')} <span class="badge badge-ok">{$t('ob_sync_recommended')}</span></span>
          <span class="ob-opt-s">{$t('ob_sync_cloud_text')}</span>
        </span>
        <span class="ob-radio" aria-hidden="true"></span>
      </button>
      <button type="button" class="ob-opt" class:on={pick === 'own'} role="radio" aria-checked={pick === 'own'} disabled={busy} onclick={() => (pick = 'own')}>
        <span class="ob-opt-ic"><Icon name="network" size={20} /></span>
        <span class="ob-opt-b">
          <span class="ob-opt-t">{$t('ob_sync_own')}</span>
          <span class="ob-opt-s">{isMobile ? $t('ob_sync_own_kinds_phone') : $t('ob_sync_own_kinds')}</span>
        </span>
        <span class="ob-radio" aria-hidden="true"></span>
      </button>
    </div>
    {#if error}<p class="ob-error" role="alert">{error}</p>{/if}
  </div>
  <div class="ob-foot">
    {#if pick === 'cloud'}
      <button type="button" class="btn btn-primary" disabled={busy} onclick={enable}>{$t('ob_sync_enable')}</button>
    {:else}
      <button type="button" class="btn btn-primary" onclick={own}>{$t('ob_sync_setup')}</button>
    {/if}
    <button type="button" class="ob-skip" disabled={busy} onclick={() => onboarding.next()}>{$t('ob_skip')}</button>
  </div>
{:else}
  <div class="ob-main">
    <span class="ob-tile"><Icon name="refresh-cw" size={28} /></span>
    <h1 class="ob-title">{$t('settings_sync_section')}</h1>
    <p class="ob-text">{$t('ob_sync_text')} {isMobile ? $t('ob_sync_own_text_phone') : $t('ob_sync_own_text')}</p>
    {#if noAccount}
      <div class="ob-note">
        <Icon name="cloud" size={18} />
        <span class="ob-note-b"><strong>{$t('ob_sync_promo')}</strong><small>{$t('ob_sync_promo_text')}</small></span>
        <button type="button" onclick={() => onboarding.go('account')}>{$t('ob_sync_promo_go')}</button>
      </div>
    {/if}
  </div>
  <div class="ob-foot">
    <button type="button" class="btn btn-primary" onclick={own}>{$t('ob_sync_setup')}</button>
    <button type="button" class="ob-skip" onclick={() => onboarding.next()}>{$t('ob_skip')}</button>
  </div>
{/if}

<style>
  .prep { position: relative; width: 112px; height: 112px; margin-bottom: var(--sp-5); }
  .core {
    position: absolute; inset: 14px; display: flex; align-items: center; justify-content: center; border-radius: 50%;
    background: var(--accent-tint); color: var(--accent); animation: breathe 2.4s ease-in-out infinite;
  }
  .ring {
    position: absolute; inset: 0; border-radius: 50%; border: 3px solid var(--accent-tint); border-top-color: var(--accent);
    animation: turn 1.1s linear infinite;
  }
  @keyframes turn { to { transform: rotate(360deg); } }
  @keyframes breathe { 0%, 100% { transform: scale(1); } 50% { transform: scale(0.93); } }
  .stages { list-style: none; margin: var(--sp-2) 0 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-3); text-align: left; }
  .stages li { display: flex; align-items: center; gap: var(--sp-3); color: var(--text-3); font-size: 15px; transition: color 0.2s ease; }
  .stages li.on { color: var(--text); font-weight: var(--fw-semibold); }
  .stages li.done { color: var(--text-2); }
  .mark { display: inline-flex; align-items: center; justify-content: center; width: 22px; height: 22px; flex-shrink: 0; border-radius: 50%; border: 1.5px solid var(--border-2); }
  .done .mark { background: var(--accent); border-color: var(--accent); color: #fff; }
  .on .mark { border-color: transparent; }
  .mark .spinner { width: 16px; height: 16px; margin: 0; }
  @media (prefers-reduced-motion: reduce) {
    .core { animation: none; }
    .ring { animation-duration: 3s; }
  }
</style>

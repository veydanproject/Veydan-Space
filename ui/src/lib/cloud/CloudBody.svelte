<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The page «Account» without its frame, the same on a computer and a phone
  (vcloud-plan.md 4.5, 4.6), from what matters most to what is rarely
  needed: who the account is, the plan in use and its room, the plans to
  choose from, how sync stands, the ways into the account (a device, the
  recovery), what the owner of the key keeps (the profile, the servers),
  the deletion at the very end. A device without an account gets the way
  to make one, and sync.
  A product on its own servers, or with no servers chosen, gets one plain
  explanation instead. `?delete=1` opens the deletion, `?section=<id>`
  scrolls to a part of the page (`plans`, `recovery-key`, `delete`), `?key=1`
  opens the key of activation (the card of the bot about a plan that ends).
-->
<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { accountSource, loadAccount, type AccountProfile } from '$lib/core/account';
  import { t, locale } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';
  import { activating, cloudPage } from '$lib/cloud/status.svelte';
  import type { CloudPlanOffer } from '$lib/cloud/api';
  import { dateOf, errorKeyOf, shownAccount, viewOf } from '$lib/cloud/account';
  import { distribution } from '$lib/core/store/distribution.svelte';
  import { planName } from '$lib/cloud/plans';
  import AccessList from '$lib/cloud/AccessList.svelte';
  import AccountHead from '$lib/cloud/AccountHead.svelte';
  import AccountLinks from '$lib/cloud/AccountLinks.svelte';
  import AccountSync from '$lib/cloud/AccountSync.svelte';
  import DeleteAccount from '$lib/cloud/DeleteAccount.svelte';
  import Notice from '$lib/cloud/Notice.svelte';
  import PlanCard from '$lib/cloud/PlanCard.svelte';
  import PlanPicker from '$lib/cloud/PlanPicker.svelte';
  import RedeemDialog from '$lib/cloud/RedeemDialog.svelte';
  import StateView from '$lib/cloud/StateView.svelte';
  import UsageCard from '$lib/cloud/UsageCard.svelte';
  import Welcome from '$lib/cloud/Welcome.svelte';

  const query = typeof window === 'undefined' ? new URLSearchParams() : new URLSearchParams(window.location.search);
  const startDelete = query.get('delete') === '1';
  const section = query.get('section');
  const startKey = query.get('key') === '1';

  /** Who the account is; undefined until the owner of the key answered, null on a device without one. */
  let profile = $state<AccountProfile | null | undefined>(accountSource() ? undefined : null);
  /** The device has no account at all: the page offers to make one, and says nothing of a cloud that cannot be asked. */
  const noAccount = $derived(accountSource() !== null && profile === null);

  onMount(() => {
    void loadAccount().then((p) => (profile = p));
    let stop: (() => void) | null = null;
    void cloudPage.listen().then((s) => (stop = s));
    void cloudPage.load().then(async () => {
      // A key is entered only where a host of the project answers for the identity.
      if (startKey && (view === 'account' || view === 'no_account')) keyOpen = true;
      if (!section) return;
      await tick();
      document.getElementById(section)?.scrollIntoView({ block: 'center' });
    });
    return () => stop?.();
  });

  const view = $derived(viewOf(cloudPage.status));
  const account = $derived(shownAccount(cloudPage.status));
  const statusError = $derived(cloudPage.status?.error ?? null);
  const plans = $derived(cloudPage.plans?.plans ?? []);
  /** There is a choice of plans on the page: more than one on offer, and a host that answers for them. */
  const canPick = $derived(plans.length > 1 && (view === 'account' || view === 'no_account'));
  /** An app from a store shows the plans only to read: no prices, no buttons (the store's payments rules). */
  const viewOnly = $derived(distribution.fromStore);
  /** The card of a frame: `card` on a computer, `m-card` on a phone. */
  const card = isMobile ? 'm-card' : 'card';

  // The key of activation: for a plan of the choice, or for any.
  let keyOpen = $state(false);
  let keyPlan = $state<CloudPlanOffer | null>(null);
  const keyResult = $derived(
    account
      ? { plan: planName(plans, account.plan), until: account.until, next: account.next ? { plan: planName(plans, account.next.plan), since: account.next.since } : null }
      : null,
  );

  function openKey(plan: CloudPlanOffer | null = null) {
    keyPlan = plan;
    keyOpen = true;
  }

  function toPlans() {
    document.getElementById('plans')?.scrollIntoView({ block: 'start', behavior: 'smooth' });
  }

  function words(code: string, message: string): string {
    const key = errorKeyOf(code);
    return key ? $t(key) : message || code;
  }

  /** The key of the dialog: a refusal is said there, so the page does not repeat it under itself. */
  async function redeem(key: string) {
    if (await cloudPage.redeem(key)) return null;
    const refused = cloudPage.error;
    cloudPage.error = null;
    return refused;
  }
</script>

<div class="cloud-body" class:m={isMobile}>
  {#if cloudPage.deleted}
    <Notice tone="ok" role="status">
      <strong>{$t('cloud_deleted_title')}</strong>
      {#if cloudPage.deleted.from_card}<span class="line">{$t('cloud_deleted_elsewhere')}</span>{/if}
      <span class="line">{$t('cloud_deleted_receipt', { receipt: cloudPage.deleted.receipt })} · {dateOf(cloudPage.deleted.deleted_at, $locale)}</span>
    </Notice>
  {/if}

  {#if profile !== undefined && accountSource()}
    <AccountHead {profile} />
  {/if}

  {#if noAccount}
    <!-- Nothing of the cloud without an account: the head offers to make one. -->
  {:else if view === 'loading'}
    <div class="{card} pad loading"><span class="spinner" aria-hidden="true"></span></div>
  {:else if view === 'own' || view === 'unchosen' || view === 'unavailable'}
    <div class="{card} pad">
      <StateView tone="info" icon="cloud" title={$t('cloud_off_title')}>
        <p>{view === 'own' ? $t('cloud_own_servers') : view === 'unchosen' ? $t('cloud_unchosen') : $t('cloud_unavailable')}</p>
        {#snippet actions()}
          {#if view === 'unchosen'}<div class="center"><a class="btn btn-primary" href="/messenger">{$t('cloud_unchosen_go')}</a></div>{/if}
        {/snippet}
      </StateView>
    </div>
  {:else}
    {#if view === 'unreachable'}
      <Notice tone="warn" icon="wifi-off">
        {statusError ? words(statusError.code, statusError.message) : $t('cloud_unreachable')}
        {#snippet action()}
          <button class="btn btn-ghost btn-sm" type="button" disabled={cloudPage.busy} onclick={() => cloudPage.load()}>{$t('cloud_retry')}</button>
        {/snippet}
      </Notice>
    {/if}

    {#if account}
      <div class="{card} pad">
        <PlanCard {account} name={planName(plans, account.plan)} nextName={account.next ? planName(plans, account.next.plan) : ''} hasPlans={canPick} onPlans={toPlans} onKey={() => openKey()} />
      </div>
      <div class="{card} pad">
        <UsageCard {account} categories={cloudPage.categories} onMore={canPick ? toPlans : () => openKey()} />
      </div>
    {:else if view === 'no_account'}
      <div class="{card} pad">
        <Welcome activation={cloudPage.activation} freeFiles={plans.find((p) => p.free)?.limits['vault.files_bytes'] ?? null} onActivate={() => cloudPage.activate()} onKey={() => openKey()} />
      </div>
    {/if}

    {#if canPick}
      <PlanPicker
        {viewOnly}
        {plans}
        current={account?.plan ?? null}
        canActivate={view === 'no_account'}
        activating={activating(cloudPage.activation)}
        onActivate={() => cloudPage.activate()}
        onKey={(plan) => openKey(plan)}
      />
    {/if}

  {/if}

  <!-- Sync is the core's: every product has it, with a cloud account or without. -->
  <AccountSync />

  {#if !noAccount && (view === 'account' || view === 'no_account' || view === 'unreachable')}
    <AccessList hasAccount={account !== null} recovery={account?.recovery ?? null} />
  {/if}

  {#if !noAccount}
    <AccountLinks />
  {/if}

  {#if !noAccount && account}
    <DeleteAccount startOpen={startDelete} />
  {/if}
  {#if cloudPage.error}
    <div class="error-msg" role="alert">{words(cloudPage.error.code, cloudPage.error.message)}</div>
  {/if}
</div>

<RedeemDialog bind:open={keyOpen} busy={cloudPage.busy} planName={keyPlan?.name ?? ''} result={keyResult} onRedeem={redeem} />

<style>
  .cloud-body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .pad { padding: var(--sp-6); }
  .m .pad { padding: var(--sp-4); }
  .loading { display: flex; justify-content: center; padding: var(--sp-8); }
  .line { display: block; }
  .center { display: flex; justify-content: center; }
</style>

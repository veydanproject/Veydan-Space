<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The choice of a plan (vcloud-plan.md 4.2, cloud-api.md 4.5): every plan
  on offer side by side with what it gives and what it costs, the plan in
  use marked, the next one up pointed at. The period and the currency are
  chosen once for all the cards, and a longer period says what it saves. A
  plan is turned on with a key of activation, so the button of a paid plan
  opens the key; nothing here takes money. On a phone the cards stack and
  one is open at a time. Read only (an app from a store, whose payments
  rules allow nothing to be bought in the app), a card shows the name and
  what the plan gives, and the note points to the site. The numbers are the server's: the app keeps no
  table of plans.
-->
<script lang="ts">
  import { t, locale } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { isMobile } from '$lib/core/platform';
  import type { CloudPlanOffer } from '$lib/cloud/api';
  import { formatBytes } from '$lib/cloud/account';
  import {
    bestSaving,
    countsByMonth,
    currenciesOf,
    defaultCurrency,
    featuresOf,
    formatMoney,
    MONTH_DAYS,
    periodsOf,
    perMonth,
    recommendedPlan,
    savingOf,
    shownPrice,
    YEAR_DAYS,
    type PlanFeature,
  } from '$lib/cloud/plans';

  let {
    plans,
    current,
    canActivate = false,
    viewOnly = false,
    activating = false,
    onActivate,
    onKey,
  }: {
    plans: CloudPlanOffer[];
    /** The id of the plan of the account, or null without an account. */
    current: string | null;
    /** No account yet: the free plan can be activated here. */
    canActivate?: boolean;
    /** Plans to read only: no prices, no period or currency, no buttons. */
    viewOnly?: boolean;
    activating?: boolean;
    onActivate: () => void;
    /** Opens the key of activation for a plan. */
    onKey: (plan: CloudPlanOffer) => void;
  } = $props();

  const currencies = $derived(currenciesOf(plans));
  const free = $derived(plans.find((p) => p.free) ?? null);
  const recommended = $derived(recommendedPlan(plans, current));

  // The choice of the person; until they choose, the shortest period and the currency of the language.
  let pickedPeriod = $state<number | null>(null);
  let pickedCurrency = $state<string | null>(null);
  let opened = $state<string | null>(null);
  const currency = $derived(pickedCurrency && currencies.includes(pickedCurrency) ? pickedCurrency : defaultCurrency(currencies, $locale));
  /** The periods the chosen currency has prices for: a switch never leads to a row of cards without a price. */
  const periods = $derived(periodsOf(plans, currency));
  const period = $derived(pickedPeriod !== null && periods.includes(pickedPeriod) ? pickedPeriod : (periods[0] ?? null));
  /** The plan open: the one tapped, else the one pointed at, else the plan in use (beside the list, or in place on a narrow page). */
  const open = $derived(opened ?? recommended ?? current ?? plans[0]?.id ?? null);
  const openPlan = $derived(plans.find((p) => p.id === open) ?? null);

  function periodLabel(days: number): string {
    if (days === MONTH_DAYS) return $t('cloud_plan_period_month');
    if (days === YEAR_DAYS) return $t('cloud_plan_period_year');
    return $t('cloud_plan_period_days', { n: String(days) });
  }

  /** What a period longer than a month costs in all: «3 490 ₽ за год». */
  function billed(amountMinor: number, days: number, code: string): string {
    const amount = formatMoney(amountMinor, code, $locale);
    return days === YEAR_DAYS ? $t('cloud_plan_billed_year', { amount }) : $t('cloud_plan_billed_days', { amount, n: String(days) });
  }

  function featureText(f: PlanFeature): string {
    switch (f.id) {
      case 'total':
        return $t('cloud_plan_f_total', { size: formatBytes(f.value) });
      case 'files':
        return $t('cloud_plan_f_files', { size: formatBytes(f.value) });
      case 'chat_file':
        return $t('cloud_plan_f_chat_file', { size: formatBytes(f.value) });
      case 'retention':
        return $t('cloud_plan_f_retention', { days: String(f.value) });
      case 'upload':
        return $t('cloud_plan_f_upload', { size: formatBytes(f.value) });
    }
  }
</script>

{#snippet top(plan: CloudPlanOffer)}
  {@const price = shownPrice(plan, currency, period)}
  {@const saved = price && price.period_days === period ? savingOf(plan, currency, period) : null}
  <span class="name-row">
    <span class="name">{plan.name}</span>
    {#if plan.id === current}<span class="badge badge-ok">{$t('cloud_plan_current')}</span>
    {:else if plan.id === recommended}<span class="badge badge-accent">{$t('cloud_plan_recommended')}</span>{/if}
  </span>
  {#if !viewOnly}
  <span class="price-row">
    {#if plan.free}
      <span class="price">{$t('cloud_plan_free_price')}</span>
    {:else if price && currency && countsByMonth(price.period_days)}
      <!-- A year is shown as what it costs a month, rounded; a month's own price is never rounded. -->
      <span class="price">{formatMoney(perMonth(price), currency, $locale, price.period_days !== MONTH_DAYS)}</span>
      <span class="per">{$t('cloud_plan_per_month')}</span>
    {:else if price && currency}
      <span class="price">{formatMoney(price.amount_minor, currency, $locale)}</span>
      <span class="per">/ {periodLabel(price.period_days)}</span>
    {:else}
      <span class="price none">{$t('cloud_plan_no_price')}</span>
    {/if}
  </span>
  <span class="bill">
    {#if plan.free}
      {$t('cloud_plan_free_forever')}
    {:else if price && currency && price.period_days > MONTH_DAYS && countsByMonth(price.period_days)}
      {billed(price.amount_minor, price.period_days, currency)}{#if saved}<span class="save">−{saved}%</span>{/if}
    {:else if price && price.period_days === MONTH_DAYS}
      {$t('cloud_plan_billed_monthly')}
    {:else}
      &nbsp;
    {/if}
  </span>
  {/if}
{/snippet}

{#snippet body(plan: CloudPlanOffer)}
  {@const mine = plan.id === current}
  <ul class="features">
    {#each featuresOf(plan, free) as f (f.id)}
      <li class:more={f.more}><Icon name="check" size={14} strokeWidth={2.5} /><span>{featureText(f)}</span></li>
    {/each}
    <li><Icon name="check" size={14} strokeWidth={2.5} /><span>{$t('cloud_plan_f_sync')}</span></li>
    <li><Icon name="check" size={14} strokeWidth={2.5} /><span>{$t('cloud_plan_f_recovery')}</span></li>
  </ul>

  {#if !viewOnly}
  <div class="cta">
    {#if plan.free && canActivate}
      <button class="btn btn-primary" type="button" disabled={activating} onclick={onActivate}>
        {activating ? $t('cloud_activating') : $t('cloud_activate')}
      </button>
    {:else if plan.free}
      <span class="quiet">{mine ? $t('cloud_plan_in_use') : $t('cloud_plan_free_after')}</span>
    {:else if mine}
      <button class="btn btn-ghost" type="button" onclick={() => onKey(plan)}>{$t('cloud_plan_extend')}</button>
    {:else}
      <!-- One plan is open at a time (beside the list, or in place): its choice is the panel's one action. The plan pointed at keeps its badge and its frame. -->
      <button class="btn btn-primary" type="button" onclick={() => onKey(plan)}>
        {$t('cloud_plan_choose', { plan: plan.name })}
      </button>
    {/if}
  </div>
  {/if}
{/snippet}

<section class="picker" class:m={isMobile} id="plans" aria-labelledby="cloud-plans-title">
  <div class="head">
    <h2 id="cloud-plans-title" class={isMobile ? 'm-label' : 'section-label'}>{$t('cloud_plans_title')}</h2>
    {#if !viewOnly && currencies.length > 1}
      <select class="currency" value={currency} onchange={(e) => (pickedCurrency = e.currentTarget.value)} aria-label={$t('cloud_plan_currency')}>
        {#each currencies as code (code)}<option value={code}>{code}</option>{/each}
      </select>
    {/if}
  </div>
  {#if !viewOnly && periods.length > 1}
    <div class="seg periods" role="radiogroup" aria-label={$t('cloud_plan_period')} style:--n={periods.length}>
      {#each periods as days (days)}
        {@const saved = bestSaving(plans, currency, days)}
        <button class="seg-btn" class:active={period === days} type="button" role="radio" aria-checked={period === days} onclick={() => (pickedPeriod = days)}>
          <span>{periodLabel(days)}</span>{#if saved}<span class="save">−{saved}%</span>{/if}
        </button>
      {/each}
    </div>
  {/if}

  <!-- The plans as a list and the chosen one beside it where the page is wide; where it is narrow, the chosen one opens in place. -->
  <div class="pmd">
    <ul class="plans">
      {#each plans as plan (plan.id)}
        <li class="plan" class:mine={plan.id === current} class:pointed={plan.id === recommended} class:open={open === plan.id}>
          <button class="top" type="button" aria-expanded={open === plan.id} onclick={() => (opened = plan.id)}>{@render top(plan)}</button>
          {#if open === plan.id}<div class="inline">{@render body(plan)}</div>{/if}
        </li>
      {/each}
    </ul>
    {#if openPlan}
      <div class="detail" class:pointed={openPlan.id === recommended} aria-live="polite">
        <div class="detail-top">{@render top(openPlan)}</div>
        {@render body(openPlan)}
      </div>
    {/if}
  </div>
  <p class="note">{viewOnly ? $t('cloud_plan_site_note') : $t('cloud_plan_key_note')}</p>
</section>

<style>
  .picker { display: flex; flex-direction: column; gap: var(--sp-3); container-type: inline-size; }
  .head { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); }
  h2 { margin: 0; }
  .section-label { padding: var(--sp-3) var(--sp-1) var(--sp-1); }
  .currency { width: auto; height: var(--control-h); font-weight: var(--fw-semibold); }
  .m .currency { height: var(--control-h-lg); }
  .save {
    margin-left: 6px; padding: 1px 6px; border-radius: var(--radius-pill);
    background: var(--success-bg); color: var(--success-text); border: 1px solid var(--success-border);
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); white-space: nowrap;
  }
  /* The periods: one row of equal parts; on a phone the saving goes under the name, so four fit. */
  .periods { display: grid; grid-template-columns: repeat(var(--n), minmax(0, 1fr)); }
  .periods .seg-btn { justify-content: center; white-space: nowrap; }
  .m .periods .seg-btn { flex-direction: column; gap: 2px; min-height: var(--sp-12); }
  .m .periods .save { margin-left: 0; }

  .pmd { display: grid; grid-template-columns: minmax(0, 1fr); gap: var(--sp-3); align-items: start; }
  .plans { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .plan { border-radius: var(--radius-lg); border: 1px solid var(--border); background: var(--surface); overflow: hidden; }
  .plan.open { border-color: var(--accent-border); }
  /* The plan the page points at carries the accent, open or not. */
  .plan.pointed { border-color: var(--accent-border); box-shadow: 0 0 0 1px var(--accent-border); }
  .top {
    display: flex; flex-direction: column; gap: var(--sp-1); width: 100%; padding: var(--sp-4);
    border: 0; background: none; color: inherit; font: inherit; text-align: left; cursor: pointer;
    transition: background var(--dur-fast);
  }
  .top:hover { background: var(--surface-row-hover); }
  /* The card clips what is outside it: the keyboard's ring drawn inside. */
  .top:focus-visible { outline: 2px solid var(--accent-border); outline-offset: -2px; }
  .inline { display: flex; flex-direction: column; gap: var(--sp-3); padding: 0 var(--sp-4) var(--sp-4); }
  .detail { display: none; }
  /* Wide: the list keeps the heads, the chosen plan opens beside it. */
  @container (min-width: 640px) {
    .pmd { grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); }
    .inline { display: none; }
    .plan.open { background: var(--accent-bg); }
    .detail {
      display: flex; flex-direction: column; gap: var(--sp-4); padding: var(--sp-6);
      border-radius: var(--radius-card); border: 1px solid var(--border); background: var(--surface);
    }
    .detail.pointed { border-color: var(--accent-border); box-shadow: 0 0 0 1px var(--accent-border); }
    .detail-top { display: flex; flex-direction: column; gap: var(--sp-1); }
    .detail-top .price { font-size: var(--fs-2xl); }
  }

  .name-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); min-height: 26px; }
  .name { font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); }
  .price-row { display: flex; align-items: baseline; gap: 6px; }
  .price { font-size: var(--fs-xl); font-weight: var(--fw-extrabold); color: var(--text); letter-spacing: -0.01em; line-height: 1.15; font-variant-numeric: tabular-nums; }
  .price.none { font-size: var(--fs-md); font-weight: var(--fw-semibold); color: var(--text-2); }
  .per { font-size: var(--fs-sm); color: var(--text-2); }
  .bill { font-size: var(--fs-sm); color: var(--text-2); min-height: 1.4em; display: flex; align-items: center; flex-wrap: wrap; font-variant-numeric: tabular-nums; }

  .features { list-style: none; margin: 0; padding: var(--sp-4) 0 0; border-top: 1px solid var(--border); display: flex; flex-direction: column; gap: var(--sp-2); }
  .features li { display: flex; align-items: flex-start; gap: var(--sp-2); font-size: var(--fs-sm); line-height: 1.4; color: var(--text-body); }
  .features li :global(svg) { flex-shrink: 0; margin-top: 2px; color: var(--success-text); }
  .features li.more { color: var(--text); font-weight: var(--fw-semibold); }
  .m .features li { font-size: var(--fs-base); }

  .cta { display: flex; }
  .cta .btn { flex: 1; justify-content: center; }
  .m .cta .btn { min-height: var(--sp-12); font-size: var(--fs-body); }
  .quiet { font-size: var(--fs-sm); color: var(--text-3); min-height: var(--control-h); display: flex; align-items: center; }
  .note { margin: var(--sp-1) var(--sp-1) 0; font-size: var(--fs-sm); color: var(--text-3); line-height: 1.45; }
</style>

<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The choice of a plan (vcloud-plan.md 4.2, cloud-api.md 4.5): every plan
  on offer side by side with what it gives and what it costs, the plan in
  use marked, the next one up pointed at. The period and the currency are
  chosen once for all the cards, and a longer period says what it saves. A
  plan is turned on with a key of activation, so the button of a paid plan
  opens the key; nothing here takes money. On a phone the cards stack and
  one is open at a time. The numbers are the server's: the app keeps no
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
    activating = false,
    onActivate,
    onKey,
  }: {
    plans: CloudPlanOffer[];
    /** The id of the plan of the account, or null without an account. */
    current: string | null;
    /** No account yet: the free plan can be activated here. */
    canActivate?: boolean;
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
  /** The card open on a phone: the one tapped, else the one pointed at, else the plan in use. */
  const open = $derived(opened ?? recommended ?? current ?? plans[0]?.id ?? null);

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

<section class="picker" class:m={isMobile} id="plans" aria-labelledby="cloud-plans-title">
  <div class="head">
    <h2 id="cloud-plans-title" class={isMobile ? 'm-label' : 'section-label'}>{$t('cloud_plans_title')}</h2>
    <div class="controls">
      {#if periods.length > 1}
        <div class="seg" role="radiogroup" aria-label={$t('cloud_plan_period')}>
          {#each periods as days (days)}
            {@const saved = bestSaving(plans, currency, days)}
            <button class="seg-btn" class:active={period === days} type="button" role="radio" aria-checked={period === days} onclick={() => (pickedPeriod = days)}>
              {periodLabel(days)}{#if saved}<span class="save">−{saved}%</span>{/if}
            </button>
          {/each}
        </div>
      {/if}
      {#if currencies.length > 1}
        <select class="currency" value={currency} onchange={(e) => (pickedCurrency = e.currentTarget.value)} aria-label={$t('cloud_plan_currency')}>
          {#each currencies as code (code)}<option value={code}>{code}</option>{/each}
        </select>
      {/if}
    </div>
  </div>

  <ul class="plans">
    {#each plans as plan (plan.id)}
      {@const mine = plan.id === current}
      {@const pointed = plan.id === recommended}
      {@const price = shownPrice(plan, currency, period)}
      {@const saved = price && price.period_days === period ? savingOf(plan, currency, period) : null}
      {@const shown = !isMobile || open === plan.id}
      <li class="plan" class:mine class:pointed class:shown>
        {#snippet top()}
        <span class="name-row">
          <span class="name">{plan.name}</span>
          {#if mine}<span class="badge badge-ok">{$t('cloud_plan_current')}</span>
          {:else if pointed}<span class="badge badge-accent">{$t('cloud_plan_recommended')}</span>{/if}
        </span>
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
        {/snippet}
        <!-- On a phone the head of a card opens it; on a computer every card is open. -->
        {#if isMobile}
          <button class="top" type="button" aria-expanded={shown} onclick={() => (opened = plan.id)}>{@render top()}</button>
        {:else}
          <div class="top">{@render top()}</div>
        {/if}

        {#if shown}
          <ul class="features">
            {#each featuresOf(plan, free) as f (f.id)}
              <li class:more={f.more}><Icon name="check" size={14} strokeWidth={2.5} /><span>{featureText(f)}</span></li>
            {/each}
            <li><Icon name="check" size={14} strokeWidth={2.5} /><span>{$t('cloud_plan_f_sync')}</span></li>
            <li><Icon name="check" size={14} strokeWidth={2.5} /><span>{$t('cloud_plan_f_recovery')}</span></li>
          </ul>

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
              <button class="btn" class:btn-primary={pointed} class:btn-ghost={!pointed} type="button" onclick={() => onKey(plan)}>
                {$t('cloud_plan_choose', { plan: plan.name })}
              </button>
            {/if}
          </div>
        {/if}
      </li>
    {/each}
  </ul>
  <p class="note">{$t('cloud_plan_key_note')}</p>
</section>

<style>
  .picker { display: flex; flex-direction: column; gap: var(--sp-2); }
  .head { display: flex; align-items: flex-end; justify-content: space-between; gap: var(--sp-3); flex-wrap: wrap; }
  h2 { margin: 0; }
  .section-label { padding: var(--sp-3) var(--sp-1) var(--sp-1); }
  .controls { display: flex; align-items: center; gap: var(--sp-2); margin-left: auto; }
  .m .controls { margin: 0 0 var(--sp-2); width: 100%; }
  .m .seg { flex: 1; }
  .m .seg-btn { flex: 1; justify-content: center; }
  .currency { width: auto; height: 44px; }
  .save {
    margin-left: 6px; padding: 1px 6px; border-radius: var(--radius-pill);
    background: var(--success-bg); color: var(--success-text); border: 1px solid var(--success-border);
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); white-space: nowrap;
  }

  .plans { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fit, minmax(210px, 1fr)); gap: var(--sp-3); align-items: stretch; }
  .m .plans { grid-template-columns: 1fr; gap: var(--sp-2); }
  .plan {
    display: flex; flex-direction: column; gap: var(--sp-4); padding: var(--sp-5);
    border-radius: var(--radius-lg); border: 1px solid var(--border); background: var(--surface);
  }
  .m .plan { padding: var(--sp-4); gap: var(--sp-3); }
  /* The plan the page points at carries the accent; the plan in use, the quiet mark of a thing already had. */
  .plan.pointed { border-color: var(--accent-border); box-shadow: 0 0 0 1px var(--accent-border); }
  .m .plan.shown:not(.pointed) { border-color: var(--border-2); }

  .top { display: flex; flex-direction: column; gap: var(--sp-1); padding: 0; border: 0; background: none; color: inherit; font: inherit; text-align: left; width: 100%; }
  .name-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); min-height: 26px; }
  .name { font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); }
  .price-row { display: flex; align-items: baseline; gap: 6px; margin-top: var(--sp-1); }
  .price { font-size: var(--fs-xl); font-weight: var(--fw-extrabold); color: var(--text); letter-spacing: -0.01em; line-height: 1.15; }
  .price.none { font-size: var(--fs-md); font-weight: var(--fw-semibold); color: var(--text-2); }
  .per { font-size: var(--fs-sm); color: var(--text-2); }
  .bill { font-size: var(--fs-sm); color: var(--text-2); min-height: 1.4em; display: flex; align-items: center; flex-wrap: wrap; }

  .features { list-style: none; margin: 0; padding: var(--sp-4) 0 0; border-top: 1px solid var(--border); display: flex; flex-direction: column; gap: var(--sp-2); flex: 1; }
  .features li { display: flex; align-items: flex-start; gap: var(--sp-2); font-size: var(--fs-sm); line-height: 1.4; color: var(--text-body); }
  .features li :global(svg) { flex-shrink: 0; margin-top: 2px; color: var(--success-text); }
  .features li.more { color: var(--text); font-weight: var(--fw-semibold); }
  .m .features li { font-size: 14px; }

  .cta { display: flex; }
  .cta .btn { flex: 1; justify-content: center; }
  .m .cta .btn { min-height: 48px; font-size: 15px; }
  .quiet { font-size: var(--fs-sm); color: var(--text-3); min-height: var(--control-h); display: flex; align-items: center; }
  .note { margin: var(--sp-1) var(--sp-1) 0; font-size: var(--fs-sm); color: var(--text-3); line-height: 1.45; }
</style>

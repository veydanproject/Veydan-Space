// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the choice of a plan makes of `cloud_plans` (cloud-api.md 4.5,
// vcloud-plan.md 4.2): the periods and currencies on offer, a price for a
// person, what a longer period saves, what a plan gives in words. The app
// keeps no table of plans: every number here is the server's.

import type { CloudPlanOffer, CloudPlanPrice } from '$lib/cloud/api';

/** The days of a month and of a year as the prices count them. */
export const MONTH_DAYS = 30;
export const YEAR_DAYS = 365;

/** The periods the plans have prices for in `currency` (in any, when none is given), the shortest first. */
export function periodsOf(plans: CloudPlanOffer[], currency: string | null = null): number[] {
  const days = new Set<number>();
  for (const plan of plans) {
    for (const price of plan.prices) if (price.period_days > 0 && (!currency || price.currency === currency)) days.add(price.period_days);
  }
  return [...days].sort((a, b) => a - b);
}

/** The currencies the plans have prices in, in the order they first appear. */
export function currenciesOf(plans: CloudPlanOffer[]): string[] {
  const seen: string[] = [];
  for (const plan of plans) for (const price of plan.prices) if (!seen.includes(price.currency)) seen.push(price.currency);
  return seen;
}

/** The currency a language most likely pays in; the choice stays the person's. */
const BY_LANGUAGE: Record<string, string> = {
  ru: 'RUB',
  de: 'EUR',
  fr: 'EUR',
  it: 'EUR',
  es: 'EUR',
  pl: 'EUR',
};

/** The currency shown first: the one of the language when it is on offer, else US dollars, else the first. */
export function defaultCurrency(currencies: string[], locale: string | undefined): string | null {
  if (currencies.length === 0) return null;
  const guess = BY_LANGUAGE[(locale ?? '').toLowerCase().split('-')[0]];
  if (guess && currencies.includes(guess)) return guess;
  return currencies.includes('USD') ? 'USD' : currencies[0];
}

/** The price of a plan in a currency for a period, or null. */
export function priceOf(plan: CloudPlanOffer, currency: string | null, period: number | null): CloudPlanPrice | null {
  if (!currency || !period) return null;
  return plan.prices.find((p) => p.currency === currency && p.period_days === period) ?? null;
}

/**
 * The price a card shows for the chosen period: the plan's own price for
 * it, else its price in the same currency for the nearest period (a plan
 * sold by the month alone still shows what it costs while «Год» is
 * chosen), else none.
 */
export function shownPrice(plan: CloudPlanOffer, currency: string | null, period: number | null): CloudPlanPrice | null {
  const exact = priceOf(plan, currency, period);
  if (exact || !currency) return exact;
  const inCurrency = plan.prices.filter((p) => p.currency === currency && p.period_days > 0);
  if (inCurrency.length === 0) return null;
  const want = period ?? MONTH_DAYS;
  return inCurrency.sort((a, b) => Math.abs(a.period_days - want) - Math.abs(b.period_days - want) || a.period_days - b.period_days)[0];
}

/**
 * Whether a code is one `Intl` writes as money (ISO 4217). The others
 * the owner may add (USDT) are counted in hundredths, the way the admin
 * shows every amount (cloud-api.md 4.5).
 */
function isoCurrency(currency: string): boolean {
  return /^[A-Z]{3}$/.test(currency) && currency !== 'BTC';
}

function digitsOf(currency: string, locale: string | undefined): number {
  if (!isoCurrency(currency)) return 2;
  try {
    return new Intl.NumberFormat(locale, { style: 'currency', currency }).resolvedOptions().maximumFractionDigits ?? 2;
  } catch {
    return 2;
  }
}

/**
 * An amount of minor units for a person: «3,99 €», “$3.99”, «400 USDT».
 * Whole amounts lose their zeros («349 ₽»); a currency `Intl` cannot
 * write keeps its code after the number. `rounded`: an amount of a
 * hundred units or more is shown whole.
 */
export function formatMoney(amountMinor: number, currency: string, locale?: string, rounded = false): string {
  const digits = digitsOf(currency, locale);
  let value = amountMinor / 10 ** digits;
  // A price derived for a month («291 ₽ в месяц») drops the small change once it stops meaning anything.
  if (rounded && value >= 100) value = Math.round(value);
  const fraction = Number.isInteger(value) ? 0 : Math.min(digits, 2);
  if (isoCurrency(currency)) {
    try {
      return new Intl.NumberFormat(locale, {
        style: 'currency',
        currency,
        minimumFractionDigits: fraction,
        maximumFractionDigits: fraction,
      }).format(value);
    } catch {
      // Not a code `Intl` takes: the number and the code.
    }
  }
  const number = new Intl.NumberFormat(locale, { minimumFractionDigits: fraction, maximumFractionDigits: Math.max(fraction, 2) }).format(value);
  return `${number} ${currency}`;
}

/** The days of an average month, and how far from whole months a period may be and still be counted in months. */
const MONTH_AVERAGE = 30.4375;
const MONTH_SLACK = 3;

/** The months a period is paid for: a year is twelve, as a person counts it. */
export function monthsOf(periodDays: number): number {
  return Math.max(1, Math.round(periodDays / MONTH_AVERAGE));
}

/**
 * Whether the price of a period is shown as what it costs a month: a
 * month, a quarter, a year. A week or 45 days keep their own price, which
 * no division makes a month's.
 */
export function countsByMonth(periodDays: number): boolean {
  return Math.abs(periodDays - monthsOf(periodDays) * MONTH_AVERAGE) <= MONTH_SLACK;
}

/** What a period costs a month, in minor units: a year's price over its twelve months. */
export function perMonth(price: CloudPlanPrice): number {
  return Math.round(price.amount_minor / monthsOf(price.period_days));
}

/**
 * How much less `period` costs than the shortest period of the same plan
 * and currency, for the same time, in whole percent; null when there is
 * nothing to compare with or nothing saved.
 */
export function savingOf(plan: CloudPlanOffer, currency: string | null, period: number | null): number | null {
  const price = priceOf(plan, currency, period);
  if (!price) return null;
  const base = plan.prices
    .filter((p) => p.currency === currency && p.period_days > 0 && p.period_days < price.period_days)
    .sort((a, b) => a.period_days - b.period_days)[0];
  if (!base || base.amount_minor <= 0) return null;
  // Two periods of a month or more are compared the way a person does it, by the month; a week against a month, by the day.
  const byMonth = countsByMonth(base.period_days) && countsByMonth(price.period_days);
  const unit = (p: CloudPlanPrice) => (byMonth ? monthsOf(p.period_days) : p.period_days);
  const saved = Math.round((1 - price.amount_minor / unit(price) / (base.amount_minor / unit(base))) * 100);
  return saved > 0 ? saved : null;
}

/** The largest saving any plan gives for `period`: the mark on the switch of the periods. */
export function bestSaving(plans: CloudPlanOffer[], currency: string | null, period: number): number | null {
  let best: number | null = null;
  for (const plan of plans) {
    const saved = savingOf(plan, currency, period);
    if (saved !== null && (best === null || saved > best)) best = saved;
  }
  return best;
}

/** A line of what a plan gives: which limit, its number, and whether it is more than the free plan's. */
export interface PlanFeature {
  id: 'files' | 'chat_file' | 'retention' | 'upload';
  value: number;
  more: boolean;
}

const FEATURE_KEYS: [PlanFeature['id'], string][] = [
  ['files', 'vault.files_bytes'],
  ['chat_file', 'media.max_file_bytes'],
  ['retention', 'media.retention_days'],
  ['upload', 'media.upload_per_day'],
];

/** What a plan gives by its limits, in the order a person reads them; a limit the plan does not carry is left out. */
export function featuresOf(plan: CloudPlanOffer, free: CloudPlanOffer | null): PlanFeature[] {
  const out: PlanFeature[] = [];
  for (const [id, key] of FEATURE_KEYS) {
    const value = plan.limits[key];
    if (!value || value <= 0) continue;
    const base = free?.limits[key] ?? 0;
    out.push({ id, value, more: !plan.free && value > base });
  }
  return out;
}

/** The name of a plan by its id: the server's, or the id with a capital. */
export function planName(plans: CloudPlanOffer[] | null | undefined, id: string): string {
  const known = plans?.find((p) => p.id === id)?.name;
  if (known) return known;
  // A plan the list does not name is shown by its id, as words: `premium_15` → «Premium 15».
  return id ? id[0].toUpperCase() + id.slice(1).replace(/_/g, ' ') : id;
}

/**
 * The plan the page points at: the first paid one above the plan in use,
 * by the owner's order. None for an account already on the last one, and
 * none for an account whose plan is not on offer (one by invitation, one
 * switched off): where it stands among the others is not known.
 */
export function recommendedPlan(plans: CloudPlanOffer[], currentId: string | null): string | null {
  const at = currentId ? plans.findIndex((p) => p.id === currentId) : -1;
  if (currentId && at < 0) return null;
  return plans.find((p, i) => !p.free && i > at)?.id ?? null;
}

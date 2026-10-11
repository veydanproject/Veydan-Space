// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { CloudPlanOffer } from '$lib/cloud/api';
import {
  bestSaving,
  currenciesOf,
  defaultCurrency,
  featuresOf,
  formatMoney,
  countsByMonth,
  periodsOf,
  perMonth,
  planName,
  priceOf,
  recommendedPlan,
  savingOf,
  shownPrice,
} from '$lib/cloud/plans';

const GiB = 1024 ** 3;
const free: CloudPlanOffer = {
  id: 'free',
  name: 'Free',
  free: true,
  period_days: 0,
  sort: 0,
  limits: { 'vault.files_bytes': 2 * GiB, 'media.max_file_bytes': 20 * 1024 ** 2, 'media.retention_days': 14, 'media.upload_per_day': 2 * GiB },
  prices: [],
};
const plus: CloudPlanOffer = {
  id: 'plus',
  name: 'Plus',
  free: false,
  period_days: 30,
  sort: 10,
  limits: { ...free.limits, 'vault.files_bytes': 50 * GiB, 'media.retention_days': 90 },
  prices: [
    { currency: 'USD', amount_minor: 399, period_days: 30 },
    { currency: 'USD', amount_minor: 3990, period_days: 365 },
    { currency: 'RUB', amount_minor: 34900, period_days: 30 },
    { currency: 'USDT', amount_minor: 400, period_days: 30 },
  ],
};
const pro: CloudPlanOffer = { ...plus, id: 'pro', name: 'Pro', sort: 20, prices: [{ currency: 'USD', amount_minor: 999, period_days: 30 }] };
const plans = [free, plus, pro];

describe('the choice of a plan', () => {
  it('reads the periods and the currencies from the prices', () => {
    expect(periodsOf(plans)).toEqual([30, 365]);
    expect(currenciesOf(plans)).toEqual(['USD', 'RUB', 'USDT']);
    expect(periodsOf([free])).toEqual([]);
    expect(periodsOf(plans, 'RUB')).toEqual([30]);
    expect(periodsOf(plans, 'EUR')).toEqual([]);
  });

  it('starts from the currency of the language when it is on offer', () => {
    expect(defaultCurrency(['USD', 'RUB'], 'ru')).toBe('RUB');
    expect(defaultCurrency(['USD', 'RUB'], 'de')).toBe('USD');
    expect(defaultCurrency(['EUR', 'RUB'], 'ja')).toBe('EUR');
    expect(defaultCurrency([], 'ru')).toBeNull();
  });

  it('finds a price and what it costs a month', () => {
    expect(priceOf(plus, 'USD', 365)?.amount_minor).toBe(3990);
    expect(priceOf(plus, 'EUR', 30)).toBeNull();
    expect(priceOf(plus, null, 30)).toBeNull();
    expect(perMonth({ currency: 'USD', amount_minor: 3990, period_days: 365 })).toBe(333);
    expect(perMonth({ currency: 'USD', amount_minor: 399, period_days: 30 })).toBe(399);
    expect(countsByMonth(30)).toBe(true);
    expect(countsByMonth(7)).toBe(false);
    expect(countsByMonth(45)).toBe(false);
    expect(countsByMonth(90)).toBe(true);
    expect(countsByMonth(365)).toBe(true);
  });

  it('shows the nearest price of the currency where the chosen period has none', () => {
    expect(shownPrice(plus, 'USD', 365)?.amount_minor).toBe(3990);
    expect(shownPrice(plus, 'RUB', 365)?.period_days).toBe(30);
    expect(shownPrice(pro, 'USD', 365)?.amount_minor).toBe(999);
    expect(shownPrice(plus, 'EUR', 30)).toBeNull();
    expect(shownPrice(free, 'USD', 30)).toBeNull();
  });

  it('counts what a longer period saves, and nothing for the shortest', () => {
    expect(savingOf(plus, 'USD', 365)).toBe(17);
    expect(savingOf(plus, 'USD', 30)).toBeNull();
    expect(savingOf(plus, 'RUB', 365)).toBeNull();
    expect(bestSaving(plans, 'USD', 365)).toBe(17);
    expect(bestSaving(plans, 'USD', 30)).toBeNull();
    // A week against a month is compared by the day: 100 a week is 429 a month.
    const weekly: CloudPlanOffer = { ...plus, prices: [{ currency: 'USD', amount_minor: 100, period_days: 7 }, { currency: 'USD', amount_minor: 300, period_days: 30 }] };
    expect(savingOf(weekly, 'USD', 30)).toBe(30);
  });

  it('writes money for a person', () => {
    expect(formatMoney(399, 'USD', 'en')).toBe('$3.99');
    expect(formatMoney(34900, 'RUB', 'en')).toContain('349');
    expect(formatMoney(34900, 'RUB', 'en')).not.toContain('.00');
    expect(formatMoney(400, 'USDT', 'en')).toBe('4 USDT');
    expect(formatMoney(450, 'USDT', 'en')).toBe('4.50 USDT');
    expect(formatMoney(29083, 'RUB', 'en', true)).toContain('291');
    expect(formatMoney(29083, 'RUB', 'en', true)).not.toContain('.');
    expect(formatMoney(333, 'USD', 'en', true)).toBe('$3.33');
    // A code that is not ISO money is counted in hundredths, as the admin shows it.
    expect(formatMoney(12000, 'BTC', 'en')).toBe('120 BTC');
  });

  it('says what a plan gives and what is more than the free one', () => {
    expect(featuresOf(free, free).every((f) => !f.more)).toBe(true);
    const of = featuresOf(plus, free);
    expect(of.map((f) => f.id)).toEqual(['files', 'chat_file', 'retention', 'upload']);
    expect(of.filter((f) => f.more).map((f) => f.id)).toEqual(['files', 'retention']);
    expect(featuresOf({ ...plus, limits: {} }, free)).toEqual([]);
  });

  it('names a plan and points at the next paid one', () => {
    expect(planName(plans, 'plus')).toBe('Plus');
    expect(planName(null, 'max')).toBe('Max');
    expect(recommendedPlan(plans, 'free')).toBe('plus');
    expect(recommendedPlan(plans, null)).toBe('plus');
    expect(recommendedPlan(plans, 'plus')).toBe('pro');
    expect(recommendedPlan(plans, 'pro')).toBeNull();
    // A plan that is not on offer (by invitation) has no place among the others.
    expect(recommendedPlan(plans, 'team')).toBeNull();
  });
});

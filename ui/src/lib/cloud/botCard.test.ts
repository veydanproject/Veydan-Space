// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The cards of the bot of Veydan Cloud in its chat (cloud-recovery.md §10,
// messenger-wire.md §3): fixed words by the type, the values of the table
// formatted by the app, at most one button, none that is dead, and nothing
// of a card shown as it came.

import { describe, expect, it } from 'vitest';
import type { TranslationKey } from '$lib/core/i18n';
import type { BotCard } from '$lib/messenger/seams/cards.svelte';
import { translations } from '$lib/cloud/i18n';
import {
  ACTION_ROUTES,
  CANCEL_AFTER_EXPIRY_SECS,
  cardLine,
  cardView,
  knownPlanName,
  REQUEST_TTL_SECS,
  type CardContext,
  type KnownCard,
  type PlanName,
} from '$lib/cloud/botCard';
import cardSource from '$lib/cloud/BotCard.svelte?raw';

const AT = 1_791_000_000;
const REQUEST = '9c'.repeat(16);

/** The key and what fills it: what a test reads instead of a language. */
const t = (key: TranslationKey, vars?: Record<string, string>) => (vars ? `${key} ${JSON.stringify(vars)}` : key);

const plus: PlanName = { id: 'plus', name: 'Plus' };

function ctx(over: Partial<CardContext> = {}): CardContext {
  return { now: AT + 60, at: AT, locale: 'en', closed: {}, plans: null, account: null, ...over };
}

const requested = (over: Partial<Extract<BotCard, { t: 'recovery.requested' }>> = {}): KnownCard => ({
  t: 'recovery.requested', request_id: REQUEST, created_at: AT, expires_at: AT + REQUEST_TTL_SECS, ...over,
});

const date = (seconds: number) => new Date(seconds * 1000).toLocaleDateString('en', { year: 'numeric', month: 'short', day: 'numeric' });

describe('the words of a card', () => {
  it('are the fixed sentence of its type', () => {
    expect(cardView(requested(), ctx(), t).text).toBe('cloud_alert_requested');
    expect(cardView({ t: 'recovery.escrow_read', at: AT }, ctx(), t).text).toBe('cloud_alert_escrow_read');
    expect(cardView({ t: 'recovery.restored', at: AT }, ctx(), t).text).toBe('cloud_alert_restored');
    expect(cardView({ t: 'account.deleted', receipt: null, at: AT }, ctx(), t).text).toBe('cloud_card_account_deleted');
    expect(cardView({ t: 'plan.expiring', plan: null, until: null }, ctx(), t).text).toBe('cloud_card_plan_expiring_soon');
    expect(cardView({ t: 'quota.90', pool: 'files', used: null, limit: null }, ctx(), t).text).toBe('cloud_card_quota {"pool":"cloud_pool_files"}');
    expect(cardView({ t: 'account.dormant', since: null, delete_after: null }, ctx(), t).text).toBe('cloud_card_dormant_soon');
  });

  it('carry the values the table names, formatted here', () => {
    const until = AT + 7 * 86_400;
    expect(cardView({ t: 'plan.expiring', plan: 'plus', until }, ctx({ plans: [plus] }), t).text)
      .toBe(`cloud_card_plan_expiring ${JSON.stringify({ plan: 'Plus', date: date(until) })}`);
    expect(cardView({ t: 'account.dormant', since: AT, delete_after: until }, ctx(), t).text)
      .toBe(`cloud_card_dormant ${JSON.stringify({ date: date(until) })}`);
    expect(cardView({ t: 'account.deleted', receipt: '7K2M-ABCD-EFGH', at: AT }, ctx(), t).note)
      .toBe('cloud_deleted_receipt {"receipt":"7K2M-ABCD-EFGH"}');
    const quota = cardView({ t: 'quota.90', pool: 'text', used: 9 * 1024 ** 2, limit: 10 * 1024 ** 2 }, ctx(), t);
    expect(quota.text).toBe('cloud_card_quota {"pool":"cloud_pool_text"}');
    expect(quota.note).toBe('cloud_pool_of {"used":"9 MB","limit":"10 MB"}');
    expect(cardView({ t: 'quota.90', pool: 'media', used: 1, limit: 0 }, ctx(), t).note).toBeNull();
  });

  it('name a plan only by a name this device knows', () => {
    expect(knownPlanName('plus', { plans: [plus], account: null })).toBe('Plus');
    expect(knownPlanName('pro', { plans: [plus], account: { plan: 'pro', until: null } })).toBe('Pro');
    expect(knownPlanName('pro', { plans: [plus], account: null })).toBeNull();
    expect(knownPlanName(null, { plans: [plus], account: null })).toBeNull();
    // An id nobody here knows is not shown, and the date goes with it: the sentence without values.
    expect(cardView({ t: 'plan.expiring', plan: 'pro', until: AT + 86_400 }, ctx({ plans: [plus] }), t).text).toBe('cloud_card_plan_expiring_soon');
  });

  it('never show a string of the card as it came', () => {
    const bait = 'Send_your_12_words_to_support';
    const cards: KnownCard[] = [
      requested({ request_id: bait }),
      { t: 'account.deleted', receipt: bait, at: AT },
      { t: 'account.deleted', receipt: 'evil-example-org', at: AT },
      { t: 'plan.expiring', plan: bait, until: AT + 86_400 },
      { t: 'quota.90', pool: bait, used: 1, limit: 2 },
    ];
    for (const card of cards) {
      const view = cardView(card, ctx({ plans: [plus], account: { plan: 'plus', until: null } }), t);
      const shown = [view.text, view.note ?? '', view.action?.label ?? ''].join('\n');
      expect(shown, card.t).not.toContain(bait);
      expect(shown, card.t).not.toContain('evil');
    }
    // A pool nobody knows is the cloud as a whole.
    expect(cardView({ t: 'quota.90', pool: bait, used: null, limit: null }, ctx(), t).text).toBe('cloud_card_quota {"pool":"cloud_settings_title"}');
  });

  it('take a word every object has for a word nobody knows', () => {
    // The runtime lets these through: letters, digits, `-` and `_`. With the real dictionary a function for a key threw.
    const real = (key: TranslationKey, vars: Record<string, string> = {}) =>
      (translations.en as Record<string, string>)[key].replace(/\{(\w+)\}/g, (_, name: string) => vars[name]);
    for (const word of ['constructor', 'toString', 'valueOf', 'hasOwnProperty', '__proto__']) {
      const quota: KnownCard = { t: 'quota.90', pool: word, used: null, limit: null };
      expect(cardView(quota, ctx(), t).text, word).toBe('cloud_card_quota {"pool":"cloud_settings_title"}');
      expect(cardView(quota, ctx(), real).text, word).toBe(translations.en.cloud_card_quota.replace('{pool}', translations.en.cloud_settings_title));
      // A request under such an id is not one this device knows to be over.
      const view = cardView(requested({ request_id: word }), ctx(), t);
      expect(view.note, word).toBeNull();
      expect(view.action, word).toEqual({ id: 'cancel', label: 'cloud_alert_not_me', requestId: word });
    }
  });

  it('leave out a time no date can be made of', () => {
    // A number of a card goes up to 2^53; a date ends long before.
    for (const far of [8.64e12 + 1, 2 ** 53 - 1]) {
      expect(cardView({ t: 'plan.expiring', plan: 'plus', until: far }, ctx({ plans: [plus] }), t).text).toBe('cloud_card_plan_expiring_soon');
      expect(cardView({ t: 'account.dormant', since: AT, delete_after: far }, ctx(), t).text).toBe('cloud_card_dormant_soon');
    }
    expect(cardView({ t: 'account.dormant', since: AT, delete_after: 8.64e12 }, ctx(), t).text).not.toContain('Invalid');
  });

  it('are drawn as text alone: no markup of the card, no link', () => {
    expect(cardSource).not.toContain('{@html');
    expect(cardSource).not.toMatch(/<a[\s>]/);
    expect(cardSource).not.toContain('href');
    expect(cardSource).not.toContain('MessageContent');
    // At most one button.
    expect(cardSource.match(/<button/g)).toHaveLength(1);
  });
});

describe('the button of a card', () => {
  it('is the one of its type, or none', () => {
    expect(cardView(requested(), ctx(), t).action).toEqual({ id: 'cancel', label: 'cloud_alert_not_me', requestId: REQUEST });
    expect(cardView({ t: 'recovery.escrow_read', at: AT }, ctx(), t).action).toEqual({ id: 'recovery', label: 'cloud_alert_open_recovery' });
    expect(cardView({ t: 'recovery.restored', at: AT }, ctx(), t).action).toBeNull();
    expect(cardView({ t: 'account.deleted', receipt: null, at: AT }, ctx(), t).action).toBeNull();
    expect(cardView({ t: 'plan.expiring', plan: 'plus', until: AT }, ctx(), t).action).toEqual({ id: 'key', label: 'cloud_key_enter' });
    expect(cardView({ t: 'quota.90', pool: 'files', used: null, limit: null }, ctx(), t).action).toEqual({ id: 'usage', label: 'cloud_quota_what' });
    expect(cardView({ t: 'account.dormant', since: AT, delete_after: null }, ctx(), t).action).toBeNull();
  });

  it('leads to a part of the page of the cloud', () => {
    expect(ACTION_ROUTES).toEqual({
      recovery: '/cloud?section=recovery-key',
      key: '/cloud?key=1',
      usage: '/cloud?section=cloud-usage-title',
    });
  });

  it('of a request is gone once the request is cancelled, here or elsewhere, or refused as over', () => {
    const here = cardView(requested(), ctx({ closed: { [REQUEST]: { state: 'cancelled', frozen_until: AT + 7 * 86_400 } } }), t);
    expect(here.action).toBeNull();
    expect(here.note).toBe(`cloud_alert_cancelled ${JSON.stringify({ date: date(AT + 7 * 86_400) })}`);
    const elsewhere = cardView(requested(), ctx({ closed: { [REQUEST]: { state: 'cancelled', frozen_until: null } } }), t);
    expect(elsewhere.action).toBeNull();
    expect(elsewhere.note).toBe('cloud_card_request_cancelled');
    const over = cardView(requested(), ctx({ closed: { [REQUEST]: { state: 'closed', frozen_until: null } } }), t);
    expect(over.action).toBeNull();
    expect(over.note).toBe('cloud_help_err_request_closed');
    // Another request cancelled says nothing of this one.
    expect(cardView(requested(), ctx({ closed: { other: { state: 'cancelled', frozen_until: null } } }), t).action).not.toBeNull();
  });

  it('of a request lives as long as the server takes the cancel: a day past its end', () => {
    const end = AT + REQUEST_TTL_SECS + CANCEL_AFTER_EXPIRY_SECS;
    // Expired a minute ago: «Это не я» still closes new requests for 7 days.
    const late = cardView(requested(), ctx({ now: AT + REQUEST_TTL_SECS + 60 }), t);
    expect(late.action?.id).toBe('cancel');
    expect(late.until).toBe(end);
    const past = cardView(requested(), ctx({ now: end }), t);
    expect(past.action).toBeNull();
    expect(past.note).toBe('cloud_recover_err_expired');
    expect(past.until).toBeNull();
    // A card that names no end: 30 minutes from when it was written.
    expect(cardView(requested({ expires_at: null }), ctx({ now: end - 1 }), t).action?.id).toBe('cancel');
    expect(cardView(requested({ expires_at: null }), ctx({ now: end }), t).action).toBeNull();
    // Nothing to cancel without the id of the request.
    expect(cardView(requested({ request_id: null }), ctx(), t).action).toBeNull();
  });

  it('of a plan that ends is gone once the plan is paid further', () => {
    const until = AT + 7 * 86_400;
    const card: KnownCard = { t: 'plan.expiring', plan: 'plus', until };
    expect(cardView(card, ctx({ account: { plan: 'plus', until } }), t).action?.id).toBe('key');
    expect(cardView(card, ctx({ account: { plan: 'plus', until: until + 30 * 86_400 } }), t).action).toBeNull();
    // What is not known does not take the button away.
    expect(cardView(card, ctx({ account: { plan: 'free', until: null } }), t).action?.id).toBe('key');
    expect(cardView({ ...card, until: null }, ctx({ account: { plan: 'plus', until: until + 1 } }), t).action?.id).toBe('key');
  });
});

describe('the line of a card in the list of chats', () => {
  it('is fixed by the type and is in both languages', () => {
    const lines: [BotCard, string][] = [
      [requested(), 'cloud_card_line_requested'],
      [{ t: 'recovery.escrow_read', at: null }, 'cloud_card_line_escrow_read'],
      [{ t: 'recovery.restored', at: null }, 'cloud_card_line_restored'],
      [{ t: 'account.deleted', receipt: null, at: null }, 'cloud_deleted_title'],
      [{ t: 'plan.expiring', plan: null, until: null }, 'cloud_card_line_plan_expiring'],
      [{ t: 'quota.90', pool: null, used: null, limit: null }, 'cloud_card_line_quota'],
      [{ t: 'account.dormant', since: null, delete_after: null }, 'cloud_card_line_dormant'],
    ];
    for (const [card, key] of lines) {
      expect(cardLine(card), card.t).toBe(key);
      expect(translations.en[key as keyof typeof translations.en], key).toBeTruthy();
      expect(translations.ru[key as keyof typeof translations.ru], key).toBeTruthy();
      // No value of a card in a line.
      expect(translations.en[key as keyof typeof translations.en]).not.toContain('{');
    }
  });

  it('is the messenger\'s neutral one for a type nobody knows', () => {
    expect(cardLine({ t: 'unknown' })).toBeNull();
  });
});

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A card of the bot of Veydan Cloud in its chat (internal/cloud-recovery.md
// §10, messenger-wire.md §3): what it says and the one thing it may do, by
// its type. The words are this dictionary's; of the card only the values
// the table names are used, each formatted here: a date, sizes, the name of
// a plan the app knows by its id, a receipt of the shape receipts have, a
// code of a sign-in (six digits), a key of activation (5×4 Crockford), a
// country (two letters named by `Intl`), a browser (a word of a list), an
// amount in the currency of the price.
// Nothing of a card is shown as it came, so a stolen key of the bot writes
// no sentence and no address through a card. No state here: the component
// and the tests read these.

import type { TranslationKey } from '$lib/core/i18n';
import type { BotCard } from '$lib/messenger/seams/cards.svelte';
import type { CloudCancelled, CloudWebState } from '$lib/cloud/api';
import { dateOf, formatBytes } from '$lib/cloud/account';
import { formatMoney, MONTH_DAYS, YEAR_DAYS } from '$lib/cloud/plans';
import { formatLoginCode, isActivationKey, isLoginCode, isWebId, originLine, webConfirmHref } from '$lib/cloud/web';

/** The cards this module words; an unknown type is the messenger's neutral card. */
export type KnownCard = Exclude<BotCard, { t: 'unknown' }>;

/** The one thing a card may do. */
export type CardAction =
  /** «Это не я — отменить»: `cloud_recovery_cancel` of the request. */
  | { id: 'cancel'; label: string; requestId: string }
  /** «Открыть восстановление»: the Recovery Key on the page of the cloud. */
  | { id: 'recovery'; label: string }
  /** «Ввести ключ активации»: the page of the cloud, whose plan takes the key. */
  | { id: 'key'; label: string }
  /** «Что занимает место»: the space on the page of the cloud. */
  | { id: 'usage'; label: string }
  /** «Открыть аккаунт»: the page of the cloud. */
  | { id: 'account'; label: string }
  /** «Это не я» of a code of a sign-in: `cloud_web_login_deny`. */
  | { id: 'web_deny'; label: string; loginId: string }
  /** «Завершить сессию» of a sign-in: `cloud_web_sessions_revoke`. */
  | { id: 'web_end'; label: string; session: string }
  /** «Подтвердить»: the screen of confirmation, the values of the card in its route. */
  | { id: 'web_confirm'; label: string; to: string }
  /** «Активировать сейчас»: the page of the cloud with the key in the dialog. */
  | { id: 'activate'; label: string; to: string };

export interface CardView {
  icon: string;
  tone: 'danger' | 'warn' | 'ok' | 'info';
  /** The sentence of the type. */
  text: string;
  /** Over the sentence, bold: the purchase of `order.key`. */
  title?: string | null;
  /** A value shown large: a code of a sign-in, a key; `copy` is what the button of copying takes. */
  value?: { text: string; copy: string | null } | null;
  /** Under the sentence: where a request came from, what was paid. */
  detail?: string | null;
  /** Under it: a value of the card (a receipt, sizes), or what became of its request. */
  note: string | null;
  action: CardAction | null;
  /** Unix seconds after which the action is gone; null: it does not go by the clock. */
  until: number | null;
}

/** What the card is shown against: the clock and what this device knows. */
/** A plan as far as a card names it: the id the bot wrote and the name this device shows for it. */
export interface PlanName {
  id: string;
  name: string;
}

export interface CardContext {
  /** Now and when the bot wrote the card, unix seconds. */
  now: number;
  at: number;
  locale?: string;
  /** Requests of recovery this device knows to be over, by `request_id` (`AlertsStore.closed`). */
  closed: Record<string, CloudCancelled>;
  /** The plans on offer, when the page read them. */
  plans: PlanName[] | null;
  /** The account as this device shows it, when it read one. */
  account: { plan: string; until: number | null } | null;
  /** What this run did to the sign-ins and sessions of the web cabinet (`WebStore`). */
  web?: { denied: Record<string, CloudWebState>; ended: Record<string, true> };
}

type Tr = (key: TranslationKey, vars?: Record<string, string>) => string;

/** A request of recovery lives 30 minutes (cloud-recovery.md §8.5): the end of one whose card names none. */
export const REQUEST_TTL_SECS = 30 * 60;
/** «Это не я» still closes new requests for a day after the request expired (cloud-api.md 5.4). */
export const CANCEL_AFTER_EXPIRY_SECS = 24 * 3600;

/** A receipt of a deletion: 12 characters of Crockford base32 in groups of 4 (cloud-api.md 1). */
const RECEIPT = /^[0-9A-HJKMNP-TV-Z]{4}(-[0-9A-HJKMNP-TV-Z]{4}){2}$/;

const POOLS: Record<string, TranslationKey> = {
  total: 'cloud_pool_total',
  files: 'cloud_pool_files',
  text: 'cloud_pool_text',
  media: 'cloud_pool_media',
};

/**
 * What a map holds under a word of a card, and never what every object has
 * (`constructor`, `toString`, `__proto__` are words like any): such a word
 * is one the map does not know.
 */
function own<T>(map: Record<string, T>, word: string | null): T | undefined {
  if (word === null) return undefined;
  // Read before asking: the map of the store is reactive, and the read is what follows it.
  const held = map[word];
  return Object.prototype.hasOwnProperty.call(map, word) ? held : undefined;
}

/** The last second a date can be made of; a number of a card may be any up to 2^53. */
const MAX_DATE_SECS = 8.64e12;

/** A time of a card a date can be shown of; null: the card is worded without it. */
function dated(seconds: number | null): number | null {
  return seconds !== null && seconds > 0 && seconds <= MAX_DATE_SECS ? seconds : null;
}

/**
 * The name of the plan a card names by its id: the server's for a plan on
 * offer, the account's own plan by its id; null for an id this device does
 * not know, which is then not shown at all.
 */
export function knownPlanName(id: string | null, ctx: Pick<CardContext, 'plans' | 'account'>): string | null {
  if (!id) return null;
  const offered = ctx.plans?.find((p) => p.id === id)?.name;
  if (offered) return offered;
  return ctx.account?.plan === id ? id[0].toUpperCase() + id.slice(1) : null;
}

/** A session of the web cabinet lives a day (cloud-api.md 16.2): its «Завершить» goes with it. */
export const WEB_SESSION_SECS = 24 * 3600;

/** «Месяц», «Год», «45 дн.»: the period of a price as the plans name it. */
function periodOf(days: number | null, t: Tr): string | null {
  if (!days || days <= 0) return null;
  if (days === MONTH_DAYS) return t('cloud_plan_period_month');
  if (days === YEAR_DAYS) return t('cloud_plan_period_year');
  return t('cloud_plan_period_days', { n: String(days) });
}

/** What was paid, in the currency of the price; null without both. */
function paidOf(amount: number | null, currency: string | null, locale?: string): string | null {
  return amount !== null && amount >= 0 && currency && /^[A-Z]{3,5}$/.test(currency) ? formatMoney(amount, currency, locale) : null;
}

/** The card as the chat shows it. */
export function cardView(card: KnownCard, ctx: CardContext, t: Tr): CardView {
  const date = (seconds: number) => dateOf(seconds, ctx.locale);
  const origin = (country: string | null, family: string | null) => originLine(country, family, t, ctx.locale);
  switch (card.t) {
    case 'recovery.requested': {
      const view: CardView = { icon: 'alert-triangle', tone: 'danger', text: t('cloud_alert_requested'), note: null, action: null, until: null };
      const over = own(ctx.closed, card.request_id);
      if (over) {
        // Cancelled here: the answer with the freeze. Elsewhere, or refused: what is known of it.
        if (over.frozen_until) view.note = t('cloud_alert_cancelled', { date: date(over.frozen_until) });
        else view.note = t(over.state === 'cancelled' ? 'cloud_card_request_cancelled' : 'cloud_help_err_request_closed');
        return view;
      }
      const end = (card.expires_at ?? ctx.at + REQUEST_TTL_SECS) + CANCEL_AFTER_EXPIRY_SECS;
      if (ctx.now >= end) {
        view.note = t('cloud_recover_err_expired');
        return view;
      }
      // Without the id of the request there is nothing to cancel.
      if (card.request_id) {
        view.action = { id: 'cancel', label: t('cloud_alert_not_me'), requestId: card.request_id };
        view.until = end;
      }
      return view;
    }
    case 'recovery.escrow_read':
      return {
        icon: 'alert-triangle', tone: 'warn', text: t('cloud_alert_escrow_read'), note: null,
        action: { id: 'recovery', label: t('cloud_alert_open_recovery') }, until: null,
      };
    case 'recovery.restored':
      return { icon: 'check', tone: 'ok', text: t('cloud_alert_restored'), note: null, action: null, until: null };
    case 'account.deleted':
      return {
        icon: 'trash-2', tone: 'info', text: t('cloud_card_account_deleted'),
        note: card.receipt && RECEIPT.test(card.receipt) ? t('cloud_deleted_receipt', { receipt: card.receipt }) : null,
        action: null, until: null,
      };
    case 'plan.expiring': {
      const plan = knownPlanName(card.plan, ctx);
      const until = dated(card.until);
      const text = plan && until ? t('cloud_card_plan_expiring', { plan, date: date(until) }) : t('cloud_card_plan_expiring_soon');
      // Paid further than the card says: the plan was extended since, the key is not asked for.
      const extended = card.until !== null && ctx.account?.until != null && ctx.account.until > card.until;
      return { icon: 'clock', tone: 'warn', text, note: null, action: extended ? null : { id: 'key', label: t('cloud_key_enter') }, until: null };
    }
    case 'quota.90': {
      const pool = t(own(POOLS, card.pool) ?? 'cloud_settings_title');
      const sized = card.used !== null && card.limit !== null && card.limit > 0 && card.used >= 0;
      return {
        icon: 'alert-triangle', tone: 'warn', text: t('cloud_card_quota', { pool }),
        note: sized ? t('cloud_pool_of', { used: formatBytes(card.used ?? 0), limit: formatBytes(card.limit ?? 0) }) : null,
        action: { id: 'usage', label: t('cloud_quota_what') }, until: null,
      };
    }
    case 'account.dormant': {
      const end = dated(card.delete_after);
      return {
        icon: 'clock', tone: 'warn',
        text: end ? t('cloud_card_dormant', { date: date(end) }) : t('cloud_card_dormant_soon'),
        note: null, action: null, until: null,
      };
    }
    case 'account.downgraded': {
      // Nothing was deleted: new files wait for room. The limit of free, when the card names one.
      const limit = card.files_bytes !== null && card.files_bytes > 0 ? card.files_bytes : null;
      return {
        icon: 'alert-triangle', tone: 'warn', text: t('cloud_card_downgraded'),
        note: limit !== null ? t('cloud_card_downgraded_limit', { limit: formatBytes(limit) }) : null,
        action: { id: 'usage', label: t('cloud_quota_what') }, until: null,
      };
    }
    case 'web.login': {
      // The words of the domain are the app's, never the card's (cloud-api.md 16.5).
      const view: CardView = {
        icon: 'key', tone: 'warn', text: t('cloud_card_web_login'), detail: origin(card.country, card.ua_family),
        note: t('cloud_card_web_login_only_site'), action: null, until: null,
      };
      const denied = isWebId(card.id) ? own(ctx.web?.denied ?? {}, card.id) : undefined;
      if (denied) {
        view.note = denied.frozen_until ? t('cloud_web_denied_frozen', { date: date(denied.frozen_until) }) : t('cloud_web_denied');
        return view;
      }
      const end = dated(card.expires_at);
      if (end !== null && ctx.now >= end) {
        // The code is dead: it is not shown, nothing is left to deny.
        view.note = t('cloud_card_web_login_expired');
        return view;
      }
      if (isLoginCode(card.code)) view.value = { text: formatLoginCode(card.code), copy: null };
      if (isWebId(card.id)) {
        view.action = { id: 'web_deny', label: t('cloud_web_not_me'), loginId: card.id };
        view.until = end;
      }
      return view;
    }
    case 'web.signed_in': {
      const at = dated(card.at);
      const view: CardView = {
        icon: 'world', tone: 'info', text: t('cloud_card_web_signed_in'),
        detail: at ? `${origin(card.country, card.ua_family)} · ${date(at)}` : origin(card.country, card.ua_family),
        note: null, action: null, until: null,
      };
      if (!isWebId(card.session)) return view;
      if (own(ctx.web?.ended ?? {}, card.session)) {
        view.note = t('cloud_web_session_ended');
        return view;
      }
      const end = at !== null ? at + WEB_SESSION_SECS : null;
      if (end !== null && ctx.now >= end) return view;
      view.action = { id: 'web_end', label: t('cloud_web_end_session'), session: card.session };
      view.until = end;
      return view;
    }
    case 'web.confirm': {
      const known = card.action === 'delete';
      const view: CardView = {
        icon: 'alert-triangle', tone: 'danger', text: t(known ? 'cloud_card_web_confirm_delete' : 'cloud_card_web_confirm'),
        detail: origin(card.country, card.ua_family), note: null, action: null, until: null,
      };
      // An action this app does not know is not confirmed from here.
      if (!known || !isWebId(card.id)) return view;
      const end = dated(card.expires_at);
      if (end !== null && ctx.now >= end) {
        view.note = t('cloud_card_web_confirm_expired');
        return view;
      }
      const to = webConfirmHref({ id: card.id, action: 'delete', country: card.country, uaFamily: card.ua_family, expiresAt: end });
      view.action = { id: 'web_confirm', label: t('cloud_web_confirm'), to };
      view.until = end;
      return view;
    }
    case 'order.paid': {
      const plan = knownPlanName(card.plan, ctx);
      const until = dated(card.until);
      const paid = paidOf(card.amount_minor, card.currency, ctx.locale);
      const period = periodOf(card.period_days, t);
      return {
        icon: 'check-circle', tone: 'ok',
        text: plan && until ? t('cloud_card_order_paid_until', { plan, date: date(until) }) : t('cloud_card_order_paid'),
        detail: [paid, period].filter(Boolean).join(' · ') || null,
        note: null, action: { id: 'account', label: t('cloud_account_open') }, until: null,
      };
    }
    case 'order.key': {
      const plan = knownPlanName(card.plan, ctx);
      const period = periodOf(card.period_days, t);
      const paid = paidOf(card.amount_minor, card.currency, ctx.locale);
      const key = isActivationKey(card.key) ? card.key : null;
      return {
        icon: 'tag', tone: 'ok', title: t('cloud_card_order_key_title'),
        text: [plan, period].filter(Boolean).join(' · ') || t('cloud_card_order_key'),
        detail: paid ? t('cloud_card_order_key_paid', { amount: paid }) : null,
        value: key ? { text: key, copy: key } : null,
        note: key ? t('cloud_card_order_key_hint') : null,
        action: key ? { id: 'activate', label: t('cloud_card_order_key_activate'), to: `/cloud?key=${key}` } : null,
        until: null,
      };
    }
  }
}

const LINES: Record<KnownCard['t'], TranslationKey> = {
  'recovery.requested': 'cloud_card_line_requested',
  'recovery.escrow_read': 'cloud_card_line_escrow_read',
  'recovery.restored': 'cloud_card_line_restored',
  'account.deleted': 'cloud_deleted_title',
  'plan.expiring': 'cloud_card_line_plan_expiring',
  'quota.90': 'cloud_card_line_quota',
  'account.dormant': 'cloud_card_line_dormant',
  'account.downgraded': 'cloud_card_line_downgraded',
  'web.login': 'cloud_card_line_web_login',
  'web.signed_in': 'cloud_card_line_web_signed_in',
  'web.confirm': 'cloud_card_line_web_confirm',
  'order.paid': 'cloud_card_line_order_paid',
  'order.key': 'cloud_card_line_order_key',
};

/** One short line of a card for the list of chats and a notice: fixed words by the type, no value of the card. */
export function cardLine(card: BotCard): TranslationKey | null {
  return card.t === 'unknown' ? null : (LINES[card.t] ?? null);
}

/** Where a card's action leads on the page of the cloud; the commands lead nowhere, and a route of its own goes with its action. */
export const ACTION_ROUTES: Record<Exclude<CardAction['id'], 'cancel' | 'web_deny' | 'web_end' | 'web_confirm' | 'activate'>, string> = {
  recovery: '/cloud?section=recovery-key',
  key: '/cloud?key=1',
  usage: '/cloud?section=cloud-usage-title',
  account: '/cloud',
};

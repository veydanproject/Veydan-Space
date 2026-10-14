// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A card of the bot of Veydan Cloud in its chat (internal/cloud-recovery.md
// §10, messenger-wire.md §3): what it says and the one thing it may do, by
// its type. The words are this dictionary's; of the card only the values
// the table names are used, each formatted here: a date, sizes, the name of
// a plan the app knows by its id, a receipt of the shape receipts have.
// Nothing of a card is shown as it came, so a stolen key of the bot writes
// no sentence and no address through a card. No state here: the component
// and the tests read these.

import type { TranslationKey } from '$lib/core/i18n';
import type { BotCard } from '$lib/messenger/seams/cards.svelte';
import type { CloudCancelled } from '$lib/cloud/api';
import { dateOf, formatBytes } from '$lib/cloud/account';

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
  | { id: 'usage'; label: string };

export interface CardView {
  icon: string;
  tone: 'danger' | 'warn' | 'ok' | 'info';
  /** The sentence of the type. */
  text: string;
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
}

type Tr = (key: TranslationKey, vars?: Record<string, string>) => string;

/** A request of recovery lives 30 minutes (cloud-recovery.md §8.5): the end of one whose card names none. */
export const REQUEST_TTL_SECS = 30 * 60;
/** «Это не я» still closes new requests for a day after the request expired (cloud-api.md 5.4). */
export const CANCEL_AFTER_EXPIRY_SECS = 24 * 3600;

/** A receipt of a deletion: 12 characters of Crockford base32 in groups of 4 (cloud-api.md 1). */
const RECEIPT = /^[0-9A-HJKMNP-TV-Z]{4}(-[0-9A-HJKMNP-TV-Z]{4}){2}$/;

const POOLS: Record<string, TranslationKey> = {
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

/** The card as the chat shows it. */
export function cardView(card: KnownCard, ctx: CardContext, t: Tr): CardView {
  const date = (seconds: number) => dateOf(seconds, ctx.locale);
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
};

/** One short line of a card for the list of chats and a notice: fixed words by the type, no value of the card. */
export function cardLine(card: BotCard): TranslationKey | null {
  return card.t === 'unknown' ? null : (LINES[card.t] ?? null);
}

/** Where a card's action leads on the page of the cloud; `cancel` leads nowhere, it is a command. */
export const ACTION_ROUTES: Record<Exclude<CardAction['id'], 'cancel'>, string> = {
  recovery: '/cloud?section=recovery-key',
  key: '/cloud?key=1',
  usage: '/cloud?section=cloud-usage-title',
};

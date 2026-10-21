// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The web cabinet of Veydan Cloud as the app speaks of it (cloud-api.md
// §16): the link of a sign-in its QR code carries, where the request came
// from in words of this app, and the routes of its screens. Nothing a site
// or the bot wrote is shown as it came: a country is two capital letters
// named by `Intl`, a browser a word of a fixed list, a code six digits.

import type { TranslationKey } from '$lib/core/i18n';

/** The domain of the cabinet: a fixed word of the app, never a value of a card or a link. */
export const CABINET_HOST = 'my.veydan.net';

/** The screen of approval of a sign-in; without an id it reads a QR code or a pasted link. */
export const WEB_LOGIN_ROUTE = '/cloud/web';
/** The screen that confirms an action the cabinet asked for. */
export const WEB_CONFIRM_ROUTE = '/cloud/web/confirm';

/** The id of a sign-in, a session, an action: 16 hex in lower case (cloud-api.md 16.2). */
export function isWebId(id: string | null | undefined): id is string {
  return typeof id === 'string' && /^[0-9a-f]{16}$/.test(id);
}

/**
 * The id of a sign-in in a scanned or pasted text: `veydan://web/<id>`
 * alone, a parameter of a newer cabinet skipped; null for anything else
 * (an address of a site included: the QR code of the cabinet carries the
 * link of the app, never a page).
 */
export function webLoginId(text: string): string | null {
  const m = /^veydan:\/\/web\/([0-9a-f]{16})(?:\?[^\s]*)?$/.exec(text.trim());
  return m ? m[1] : null;
}

/** The route of the approval of the sign-in `id`. */
export function webLoginHref(id: string): string {
  return `${WEB_LOGIN_ROUTE}?id=${id}`;
}

/** What the screen of confirmation is told of the request: none of it is sent anywhere. */
export interface WebConfirmAsk {
  id: string;
  action: 'delete';
  country: string | null;
  uaFamily: string | null;
  expiresAt: number | null;
}

/** The route of the confirmation, the values of the card already held to their shape. */
export function webConfirmHref(ask: WebConfirmAsk): string {
  const q = new URLSearchParams({ id: ask.id, action: ask.action });
  if (ask.country && isCountry(ask.country)) q.set('country', ask.country);
  if (ask.uaFamily && isUaFamily(ask.uaFamily)) q.set('ua', ask.uaFamily);
  if (ask.expiresAt) q.set('until', String(ask.expiresAt));
  return `${WEB_CONFIRM_ROUTE}?${q}`;
}

/** The confirmation a route asks for; null when its id or action is not one. */
export function webConfirmOf(search: string): WebConfirmAsk | null {
  const q = new URLSearchParams(search);
  const id = q.get('id');
  if (!isWebId(id) || q.get('action') !== 'delete') return null;
  const country = q.get('country');
  const ua = q.get('ua');
  const until = Number(q.get('until'));
  return {
    id,
    action: 'delete',
    country: country && isCountry(country) ? country : null,
    uaFamily: ua && isUaFamily(ua) ? ua : null,
    expiresAt: Number.isSafeInteger(until) && until > 0 ? until : null,
  };
}

export function isCountry(code: string): boolean {
  return /^[A-Z]{2}$/.test(code);
}

const BROWSERS: Record<string, TranslationKey> = {
  chrome: 'cloud_web_ua_chrome',
  firefox: 'cloud_web_ua_firefox',
  safari: 'cloud_web_ua_safari',
  edge: 'cloud_web_ua_edge',
  opera: 'cloud_web_ua_opera',
  other: 'cloud_web_ua_other',
};

export function isUaFamily(word: string): boolean {
  return Object.prototype.hasOwnProperty.call(BROWSERS, word);
}

/** The words of a family of browsers; one not in the list is «другой браузер». */
export function browserKey(family: string | null | undefined): TranslationKey {
  return family && isUaFamily(family) ? BROWSERS[family] : 'cloud_web_ua_other';
}

/** The name of a country in the language of the app; null when there is none to give. */
export function countryName(code: string | null | undefined, locale?: string): string | null {
  if (!code || !isCountry(code)) return null;
  try {
    const name = new Intl.DisplayNames(locale ? [locale] : undefined, { type: 'region' }).of(code);
    return name && name !== code ? name : code;
  } catch {
    return code;
  }
}

type Tr = (key: TranslationKey, vars?: Record<string, string>) => string;

/** «Германия, Chrome» or «Chrome» alone: where a request came from. */
export function originLine(country: string | null | undefined, family: string | null | undefined, t: Tr, locale?: string): string {
  const browser = t(browserKey(family));
  const place = countryName(country, locale);
  return place ? t('cloud_web_origin', { country: place, browser }) : browser;
}

/** A code of a sign-in: exactly six digits, as text. */
export function isLoginCode(code: string | null | undefined): code is string {
  return typeof code === 'string' && /^[0-9]{6}$/.test(code);
}

/** «012 345»: a code read in two halves. */
export function formatLoginCode(code: string): string {
  return `${code.slice(0, 3)} ${code.slice(3)}`;
}

/** A key of activation: 5 groups of 4 Crockford characters (cloud-api.md 16.6). */
export function isActivationKey(key: string | null | undefined): key is string {
  return typeof key === 'string' && /^[0-9A-HJKMNP-TV-Z]{4}(-[0-9A-HJKMNP-TV-Z]{4}){4}$/.test(key);
}

/** Seconds left until `until`, never below zero. */
export function secondsLeft(until: number, now: number): number {
  return Math.max(0, Math.floor(until - now));
}

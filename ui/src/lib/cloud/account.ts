// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the page of Veydan Cloud makes of `cloud_status`: which screen, the
// pools of the account, the words of a refusal, the form of the deletion
// (internal/vcloud-plan.md 4.2, 4.5, 4.6). No state here: the store and
// the components read these.

import type { CloudAccount, CloudAccountState, CloudAccountUsage, CloudKeptAccount, CloudStatus } from '$lib/cloud/api';

/** The screen the page shows. */
export type CloudView =
  | 'loading'
  /** The product offers no way to the project (or a plain browser). */
  | 'unavailable'
  /** The messenger's servers are not chosen yet. */
  | 'unchosen'
  /** The user's own servers: Veydan Cloud is not asked anything. */
  | 'own'
  /** No host answered; what the device keeps is shown, if anything. */
  | 'unreachable'
  /** A host answered: the identity has no account. */
  | 'no_account'
  | 'account';

export function viewOf(status: CloudStatus | null): CloudView {
  if (!status) return 'loading';
  const servers = status.servers ?? (status.configured ? 'veydan' : 'none');
  if (servers === 'own') return 'own';
  if (servers === 'unchosen') return 'unchosen';
  if (status.account) return 'account';
  if (status.configured) return 'no_account';
  return servers === 'veydan' ? 'unreachable' : 'unavailable';
}

/** What the screens show of an account: the server's, or the one kept here. */
export interface ShownAccount {
  state: CloudAccountState;
  plan: string;
  until: number | null;
  grace_until: number | null;
  /** The plan that waits for the end of this one; only the server's answer knows it. */
  next: CloudAccount['next'];
  limits: Record<string, number>;
  usage: CloudAccountUsage | null;
  recovery: CloudAccount['recovery'];
  /** Unix seconds of the server's answer kept on this device; null for a live one. */
  keptAt: number | null;
}

export function shownAccount(status: CloudStatus | null): ShownAccount | null {
  const live = status?.account;
  if (live) {
    return {
      state: live.state,
      plan: live.plan,
      until: live.until,
      grace_until: live.grace_until ?? null,
      next: live.next ?? null,
      limits: live.limits ?? {},
      usage: live.usage ?? null,
      recovery: live.recovery ?? null,
      keptAt: null,
    };
  }
  const kept: CloudKeptAccount | null | undefined = status?.kept;
  if (!kept) return null;
  return {
    state: kept.state,
    plan: kept.plan,
    until: kept.until,
    grace_until: kept.grace_until,
    next: null,
    limits: kept.limits ?? {},
    usage: kept.usage,
    recovery: null,
    keptAt: kept.refreshed_at,
  };
}

/** A pool of the account: the files and the text of the vault, the media of chats. */
export interface PoolView {
  id: 'files' | 'text' | 'media';
  used: number;
  /** None: no limit (the media of chats go by their age). */
  limit: number | null;
  /** 0-100, none without a limit. */
  percent: number | null;
}

function pool(id: PoolView['id'], used: number | undefined, limit: number | undefined): PoolView {
  const u = used ?? 0;
  const l = limit && limit > 0 ? limit : null;
  return { id, used: u, limit: l, percent: l ? Math.min(100, Math.round((u / l) * 100)) : null };
}

/** The pools by the keys of the contract (cloud-api.md 4.3); the breakdown by category is the vault's own page. */
export function poolsOf(account: ShownAccount | null): PoolView[] {
  if (!account) return [];
  const { limits, usage } = account;
  return [
    pool('files', usage?.vault?.files_bytes, limits['vault.files_bytes']),
    pool('text', usage?.vault?.text_bytes, limits['vault.text_bytes']),
    pool('media', usage?.media?.storage_bytes, limits['media.storage_bytes']),
  ];
}

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

/** Bytes for a person: 1024 a step, one decimal under 10 of a unit. */
export function formatBytes(n: number): string {
  let value = Math.max(0, n);
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  const text = unit > 0 && value < 10 ? value.toFixed(1).replace(/\.0$/, '') : String(Math.round(value));
  return `${text} ${UNITS[unit]}`;
}

/**
 * A date of unix seconds in the language of the page. Russian and Ukrainian
 * end a year with «г.» / «р.»: left out, since a date often ends a sentence
 * and gave «до 18 окт. 2026 г..».
 */
export function dateOf(seconds: number, locale?: string): string {
  const text = new Date(seconds * 1000).toLocaleDateString(locale, { year: 'numeric', month: 'short', day: 'numeric' });
  // \u0433 and \u0440: the Cyrillic letters of the two marks.
  return text.replace(/\s*[\u0433\u0440]\.$/u, '');
}

/**
 * A key of activation as the server reads it (cloud-api.md 4.2): `XXXX-XXXX-XXXX-XXXX-XXXX`, or null.
 * Anything but ASCII is refused before the upper case, as the module does
 * (`normalize_key`): JavaScript would make `ß` into `SS` and `ı` into `I`.
 */
export function normalizeKey(text: string): string | null {
  const alphabet = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';
  if (/[^\x00-\x7f]/.test(text)) return null;
  let plain = '';
  for (const raw of text.toUpperCase()) {
    if (raw === '-' || raw === ' ' || raw === '\t') continue;
    const c = raw === 'O' ? '0' : raw === 'I' || raw === 'L' ? '1' : raw === 'U' ? 'V' : raw;
    if (!alphabet.includes(c)) return null;
    plain += c;
  }
  if (plain.length !== 20) return null;
  return plain.match(/.{4}/g)!.join('-');
}

/** The word the deletion asks for (vcloud-plan.md 4.6). */
export const DELETE_WORD = 'DELETE';

/** Whether the deletion can ask the trusted people to drop their shares (Social Recovery, stage C5). */
export const SHARES_REVOCABLE = true;

/** The form of the deletion. */
export interface DeleteForm {
  word: string;
  /** The secret of the lock, when a lock is set. */
  secret: string;
  lockOn: boolean;
  /** Also delete the identity and the data of the messenger on this device. */
  wipeDevice: boolean;
  /** Ask the trusted people to drop their shares (only when {@link SHARES_REVOCABLE}). */
  revokeShares: boolean;
}

export function emptyDeleteForm(lockOn: boolean): DeleteForm {
  return { word: '', secret: '', lockOn, wipeDevice: false, revokeShares: SHARES_REVOCABLE };
}

/** Whether the button may send: the word typed, the lock's secret when there is a lock. */
export function deleteReady(form: DeleteForm): boolean {
  return form.word.trim() === DELETE_WORD && (!form.lockOn || form.secret.length > 0);
}

/** The words of the page by the code of a refusal; any other code shows its message. */
export const ERROR_KEYS = {
  key_malformed: 'cloud_err_key_malformed',
  key_unknown: 'cloud_err_key_unknown',
  key_used: 'cloud_err_key_used',
  key_expired: 'cloud_err_key_expired',
  key_revoked: 'cloud_err_key_revoked',
  rate_limited: 'cloud_err_rate_limited',
  deletion_pending: 'cloud_err_deletion_pending',
  no_host: 'cloud_err_no_host',
  transport: 'cloud_err_no_host',
  auth_expired: 'cloud_err_clock',
  suspended: 'cloud_err_suspended',
  lock_wrong: 'cloud_err_lock_wrong',
  locked: 'cloud_err_locked',
  confirm_required: 'cloud_err_confirm',
  no_identity: 'cloud_err_no_identity',
  messenger_stopped: 'cloud_err_messenger_stopped',
} as const;

export type CloudErrorKey = (typeof ERROR_KEYS)[keyof typeof ERROR_KEYS];

/** The key of the words for `code`, or null when the page shows the message. */
export function errorKeyOf(code: string): CloudErrorKey | null {
  return (ERROR_KEYS as Record<string, CloudErrorKey>)[code] ?? null;
}

/** A key of activation while it is typed: upper case, groups of four with dashes, 20 characters at most. */
export function formatKeyInput(text: string): string {
  const plain = text.toUpperCase().replace(/[^0-9A-Z]/g, '').slice(0, 20);
  return plain.match(/.{1,4}/g)?.join('-') ?? '';
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { AppError } from '$lib/core/types';

export function formatError(e: unknown): string {
  if (e != null && typeof e === 'object' && 'message' in e) {
    return (e as AppError).message;
  }
  return String(e);
}

/** True when the backend error carries the given `AppError.code`. */
export function hasErrorCode(e: unknown, code: AppError['code']): boolean {
  return e != null && typeof e === 'object' && (e as AppError).code === code;
}

/**
 * Detects the HOST_KEY_MISMATCH marker emitted by the backend when an SSH
 * server's pinned host key changed (terminal + SFTP connects). Returns the
 * newly received fingerprint so the UI can offer to trust it, or null.
 * Mirrors `HOST_KEY_MISMATCH_MARKER` in apps/space/src/commands/ssh.rs.
 */
export function parseHostKeyMismatch(error: string | null | undefined): string | null {
  if (!error) return null;
  const m = /HOST_KEY_MISMATCH:(SHA256:[A-Za-z0-9+/=]+)/.exec(error);
  return m ? m[1] : null;
}

/** Human-readable byte size, e.g. "1.4 MB". Binary multiplier, short units. */
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) return '';
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = bytes;
  let i = -1;
  do {
    v /= 1024;
    i++;
  } while (v >= 1024 && i < units.length - 1);
  return `${v >= 100 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

// ── Date/time formatting ─────────────────────────────────────────────────────
// Helpers follow the app UI locale ('en' | 'ru' from i18n). Callers pass the
// current `$locale`; we map it to a BCP-47 tag for Intl. Previously each screen
// re-implemented these with divergent (and sometimes hardcoded 'ru') formats.

const LOCALE_TAG: Record<string, string> = { en: 'en-US', ru: 'ru-RU' };

function tag(locale: string | undefined): string {
  return (locale && LOCALE_TAG[locale]) || locale || 'en-US';
}

function toDate(value: string | number | null | undefined): Date | null {
  if (value == null || value === '') return null;
  const d = new Date(value);
  return isNaN(d.getTime()) ? null : d;
}

/** Date only, e.g. "05.07.2026" / "7/5/2026". Empty string for null/invalid. */
export function formatDate(iso: string | null | undefined, locale: string): string {
  const d = toDate(iso);
  return d ? d.toLocaleDateString(tag(locale)) : '';
}

/** Date + time to the minute, e.g. "05.07.2026, 14:30" / "7/5/2026, 2:30 PM". Accepts ISO strings or epoch ms. Empty string for null/invalid. */
export function formatDateTime(value: string | number | null | undefined, locale: string): string {
  const d = toDate(value);
  return d
    ? d.toLocaleString(tag(locale), { year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: '2-digit' })
    : '';
}

/** Time only, e.g. "14:30". Empty string for null/invalid. */
export function formatTime(iso: string | null | undefined, locale: string): string {
  const d = toDate(iso);
  return d ? d.toLocaleTimeString(tag(locale), { hour: '2-digit', minute: '2-digit' }) : '';
}

/** UNIX-epoch (seconds) → localized date. Empty string for null/invalid. */
export function formatEpochDate(seconds: number | null | undefined, locale: string): string {
  if (seconds == null) return '';
  const d = toDate(seconds * 1000);
  return d ? d.toLocaleDateString(tag(locale)) : '';
}

/**
 * Relative time ("2 minutes ago", "just now"), localized via Intl. Falls back
 * to an absolute date once older than 7 days. Empty string for null/invalid.
 */
export function relTime(iso: string | null | undefined, locale: string): string {
  const d = toDate(iso);
  if (!d) return '';
  const diffMs = d.getTime() - Date.now();
  const abs = Math.abs(diffMs);
  const MIN = 60_000, HOUR = 3_600_000, DAY = 86_400_000;
  const rtf = new Intl.RelativeTimeFormat(tag(locale), { numeric: 'auto' });
  if (abs < MIN) return rtf.format(Math.round(diffMs / 1000), 'second');
  if (abs < HOUR) return rtf.format(Math.round(diffMs / MIN), 'minute');
  if (abs < DAY) return rtf.format(Math.round(diffMs / HOUR), 'hour');
  if (abs < 7 * DAY) return rtf.format(Math.round(diffMs / DAY), 'day');
  return d.toLocaleDateString(tag(locale));
}

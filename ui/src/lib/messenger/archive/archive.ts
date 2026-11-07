// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the screens of the archive say, as pure functions of the numbers the
// runtime sends (messenger-archive-plan.md section 5).

import type { ArchiveProgress, PauseReason } from './types';

/** Step B holds the first screen for this long at most; the restore goes on behind it (plan D9). */
export const GATE_BUDGET_MS = 20_000;

/** How far the restore is, 0..100: by bytes, by segments when the bytes are not known yet. */
export function percentOf(p: ArchiveProgress): number {
  const ratio = p.bytes_total > 0 ? p.bytes_done / p.bytes_total : p.total > 0 ? p.done / p.total : 0;
  const percent = Math.round(Math.min(1, Math.max(0, ratio)) * 100);
  // Never 100 while segments are still coming: a full line that goes on looks stuck.
  return p.phase === 'restoring' ? Math.min(percent, 99) : percent;
}

/** Seconds left from the rate of bytes since `since`, or null while there is too little to tell. */
export function etaSeconds(p: ArchiveProgress, since: { at: number; bytes: number } | null, now: number): number | null {
  if (!since || p.bytes_total <= 0) return null;
  const seconds = (now - since.at) / 1000;
  const moved = p.bytes_done - since.bytes;
  if (seconds < 1 || moved <= 0) return null;
  return Math.max(1, Math.ceil((p.bytes_total - p.bytes_done) / (moved / seconds)));
}

/** From this many seconds the estimate is said in minutes. */
export const ETA_MINUTES_FROM = 90;

/** The estimate as the screen says it: seconds up to a minute and a half, whole minutes beyond. */
export function etaParts(seconds: number): { unit: 's' | 'min'; n: number } {
  return seconds >= ETA_MINUTES_FROM ? { unit: 'min', n: Math.ceil(seconds / 60) } : { unit: 's', n: seconds };
}

/** The scope of the archive a chat's events are in (as `sync::scope_of_chat` names it): `dm`, `dm@<server>`, `group:<id>`; null for an id of another shape. */
export function scopeOfChat(chatId: string): string | null {
  const dm = /^dm:[0-9a-f]{64}(?:@([^@/]+))?$/i.exec(chatId);
  if (dm) return dm[1] ? `dm@${dm[1]}` : 'dm';
  const group = chatId.startsWith('group:') ? chatId.slice('group:'.length) : '';
  return group ? `group:${group}` : null;
}

/** `yyyy-mm` to its parts, or null for anything else. */
function monthParts(month: string | null | undefined): { year: number; month: number } | null {
  const m = month ? /^(\d{4})-(0[1-9]|1[0-2])$/.exec(month) : null;
  return m ? { year: Number(m[1]), month: Number(m[2]) } : null;
}

/** «октябрь 2026 г.»: Russian and Ukrainian end a year with a mark that is left out, since a date often ends a sentence (\u0433 and \u0440: its Cyrillic letters). */
function withoutYearMark(text: string): string {
  return text.replace(/\s*[\u0433\u0440]\.$/u, '');
}

/** «18 окт. 2026»: a date of unix seconds in the language of the page (the day a read-only storage opens again). */
export function dateOf(seconds: number, locale?: string): string {
  return withoutYearMark(new Date(seconds * 1000).toLocaleDateString(locale, { year: 'numeric', month: 'short', day: 'numeric' }));
}

/** «октябрь 2026»: the month as a period ends, in the language of the page. */
export function monthLabel(month: string | null | undefined, locale?: string): string | null {
  const p = monthParts(month);
  if (!p) return null;
  return withoutYearMark(new Intl.DateTimeFormat(locale, { month: 'long', year: 'numeric', timeZone: 'UTC' }).format(new Date(Date.UTC(p.year, p.month - 1, 1))));
}

/** «2024–2025», or «2025» for one year; null when the archive has no months yet. */
export function yearsLabel(from: string | null | undefined, to: string | null | undefined): string | null {
  const a = monthParts(from);
  const b = monthParts(to) ?? a;
  if (!a || !b) return null;
  return a.year === b.year ? String(a.year) : `${Math.min(a.year, b.year)}–${Math.max(a.year, b.year)}`;
}

/** The key of the card's state line. */
export type ArchiveStateKey =
  | 'writing'
  | 'restoring'
  | 'idle'
  | 'paused_read_only'
  | 'paused_read_only_until'
  | 'paused_quota'
  | 'paused_offline'
  | 'unavailable';

export function stateOf(available: boolean, p: ArchiveProgress, paused: PauseReason | null): ArchiveStateKey {
  if (!available) return 'unavailable';
  const reason = paused ?? (p.phase === 'paused' ? (p.reason ?? null) : null);
  if (reason) {
    switch (reason.why) {
      case 'read_only': return reason.until ? 'paused_read_only_until' : 'paused_read_only';
      case 'quota': return 'paused_quota';
      case 'offline': return 'paused_offline';
      case 'no_storage': return 'unavailable';
    }
  }
  if (p.phase === 'restoring') return 'restoring';
  if (p.phase === 'writing') return 'writing';
  return 'idle';
}

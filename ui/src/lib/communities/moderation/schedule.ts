// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/moderation/schedule.ts at 8ce09c39, then changed for channels.

// The hours of a channel and the spans of a moderator's choices, as plain
// functions: the screens word them, the tests pin them. Days run from 1
// (Monday) to 7, as the group's settings carry them; times are `HH:MM` on
// the clock of the group's zone, and `from` after `to` is a night.

import type { ScheduleView as Schedule } from '../api';

export const DAYS = [1, 2, 3, 4, 5, 6, 7] as const;

const HOUR = 3600;
const DAY = 24 * HOUR;

/** The names of the days in a language, Monday first. 5 January 2026 was a Monday. */
export function weekdayNames(tag: string, width: 'short' | 'narrow' = 'short'): string[] {
  const f = new Intl.DateTimeFormat(tag, { weekday: width, timeZone: 'UTC' });
  return DAYS.map((d) => f.format(new Date(Date.UTC(2026, 0, 4 + d))));
}

/** The zone of this device, the one a new schedule starts with. */
export function deviceZone(): string {
  try { return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'; }
  catch { return 'UTC'; }
}

/** Every zone the platform knows, sorted, with the ones given in it whatever the platform says. */
export function zoneList(...must: string[]): string[] {
  let all: string[] = [];
  try { all = Intl.supportedValuesOf('timeZone'); } catch { /* an old engine: the given ones alone */ }
  return [...new Set([...all, 'UTC', ...must.filter(Boolean)])].sort();
}

export const validTime = (hhmm: string): boolean => /^([01]\d|2[0-3]):[0-5]\d$/.test(hhmm);

/** A schedule the channel can be given: some day, two real times, and not an empty window. */
export function scheduleValid(s: Schedule): boolean {
  return s.days.length > 0 && s.days.every((d) => d >= 1 && d <= 7) && validTime(s.from) && validTime(s.to) && s.from !== s.to && !!s.tz;
}

export const everyDay = (days: number[]): boolean => DAYS.every((d) => days.includes(d));

/** `Mon–Fri`, `Mon, Wed, Fri`, `Sat, Sun`: a run of three and more days is a span. */
export function daysText(days: number[], names: string[]): string {
  const sorted = [...new Set(days)].filter((d) => d >= 1 && d <= 7).sort((a, b) => a - b);
  const runs: number[][] = [];
  for (const d of sorted) {
    const run = runs[runs.length - 1];
    if (run && run[run.length - 1] === d - 1) run.push(d);
    else runs.push([d]);
  }
  return runs
    .map((r) => (r.length >= 3 ? `${names[r[0] - 1]}–${names[r[r.length - 1] - 1]}` : r.map((d) => names[d - 1]).join(', ')))
    .join(', ');
}

/** `Mon–Fri 09:00–21:00`; the days are left out when it is every day. */
export function scheduleText(s: Schedule, names: string[]): string {
  const hours = `${s.from}–${s.to}`;
  return everyDay(s.days) ? hours : `${daysText(s.days, names)} ${hours}`;
}

/**
 * When a closed group opens, as the strip says it: the time on the group's
 * clock, and the day when it is not today there. `null` when there is
 * nothing to say: no time, a time gone, or one too far to be an opening of
 * the hours (the mode «managers only» has none).
 */
export function opensAt(at: number | null | undefined, now: number, tag: string, tz?: string): { time: string; day: string | null } | null {
  if (!at || at <= now || at - now > 7 * DAY) return null;
  const zone = tz && knownZone(tz) ? tz : undefined;
  const time = new Intl.DateTimeFormat(tag, { hour: '2-digit', minute: '2-digit', timeZone: zone }).format(at * 1000);
  const dayOf = (t: number) => new Intl.DateTimeFormat('en-CA', { year: 'numeric', month: '2-digit', day: '2-digit', timeZone: zone }).format(t * 1000);
  const day = dayOf(at) === dayOf(now) ? null : new Intl.DateTimeFormat(tag, { weekday: 'short', timeZone: zone }).format(at * 1000);
  return { time, day };
}

function knownZone(tz: string): boolean {
  try { new Intl.DateTimeFormat('en', { timeZone: tz }); return true; }
  catch { return false; }
}

/** How long a member is kept from writing. */
export type MuteSpan = 'hour' | 'day' | 'week' | 'forever';
export const MUTE_SPANS: MuteSpan[] = ['hour', 'day', 'week', 'forever'];

/** The end of a mute; `undefined` for one until lifted. */
export function muteUntil(span: MuteSpan, now: number): number | undefined {
  switch (span) {
    case 'hour': return now + HOUR;
    case 'day': return now + DAY;
    case 'week': return now + 7 * DAY;
    case 'forever': return undefined;
  }
}

/** How far back «hide everything from the author» reaches. */
export type HideSpan = 'hour' | 'day' | 'all';
export const HIDE_SPANS: HideSpan[] = ['hour', 'day', 'all'];

/** The start of that reach: `0` is from the beginning. */
export function hideFrom(span: HideSpan, now: number): number {
  return span === 'hour' ? now - HOUR : span === 'day' ? now - DAY : 0;
}

/** A period of the queue's filter. */
export type QueuePeriod = 'today' | 'week' | 'all';
export const QUEUE_PERIODS: QueuePeriod[] = ['today', 'week', 'all'];

/** The start of a period on this device's clock; `undefined` for all of it. */
export function periodSince(period: QueuePeriod, now: Date): number | undefined {
  if (period === 'all') return undefined;
  if (period === 'week') return Math.floor(now.getTime() / 1000) - 7 * DAY;
  const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  return Math.floor(midnight.getTime() / 1000);
}

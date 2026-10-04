// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Time formatting shared by chat list, bubbles and later groups. Dates
// speak the language of the app, not the one of the system: a Russian
// phone with the app in English shows "Sat", not "сб".

import { get } from 'svelte/store';
import { locale } from '$lib/core/i18n';

/** The app's language as Intl reads it. */
export const lang = (): string => get(locale);

const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

/** `14:05` */
export function clock(unix: number): string {
  return new Date(unix * 1000).toLocaleTimeString(lang(), { hour: '2-digit', minute: '2-digit' });
}

/** Chat list stamp: time today, weekday within a week, date otherwise. */
export function listStamp(unix: number | null, now = new Date()): string {
  if (!unix) return '';
  const d = new Date(unix * 1000);
  if (sameDay(d, now)) return clock(unix);
  const days = (now.getTime() - d.getTime()) / 86_400_000;
  if (days < 7) return d.toLocaleDateString(lang(), { weekday: 'short' });
  return d.toLocaleDateString(lang(), { day: '2-digit', month: '2-digit', year: '2-digit' });
}

/** `29 Sep 2026, 14:05`: a moment worth its date. */
export function stamp(unix: number): string {
  return new Date(unix * 1000).toLocaleString(lang(), { day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' });
}

/** `29 Sep 2026`: a day worth its date. */
export function date(unix: number): string {
  return new Date(unix * 1000).toLocaleDateString(lang(), { day: 'numeric', month: 'short', year: 'numeric' });
}

/** `14:05:09`: a moment of a log line. */
export function clockSeconds(ms: number): string {
  return new Date(ms).toLocaleTimeString(lang(), { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

/** `September 2026`: a heading over what was sent that month. */
export function monthTitle(unix: number): string {
  return new Date(unix * 1000).toLocaleDateString(lang(), { month: 'long', year: 'numeric' });
}

/** Day key for separators. */
export function dayKey(unix: number): string {
  const d = new Date(unix * 1000);
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
}

/** `today` / `yesterday` markers are returned as keys for i18n. */
export function dayLabel(unix: number, now = new Date()): { key: 'today' | 'yesterday' | null; text: string } {
  const d = new Date(unix * 1000);
  if (sameDay(d, now)) return { key: 'today', text: '' };
  const y = new Date(now);
  y.setDate(now.getDate() - 1);
  if (sameDay(d, y)) return { key: 'yesterday', text: '' };
  const opts: Intl.DateTimeFormatOptions =
    d.getFullYear() === now.getFullYear()
      ? { day: 'numeric', month: 'long' }
      : { day: 'numeric', month: 'long', year: 'numeric' };
  return { key: null, text: d.toLocaleDateString(lang(), opts) };
}

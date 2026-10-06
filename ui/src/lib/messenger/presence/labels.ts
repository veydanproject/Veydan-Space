// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the line under a contact's name says of their presence. Pure: the
// store passes the time, the tests pass any time they like.

import { lang } from '../shared/time';

export type PresenceKey =
  | 'msg_presence_online'
  | 'msg_presence_just_now'
  | 'msg_presence_minutes'
  | 'msg_presence_hours'
  | 'msg_presence_yesterday'
  | 'msg_presence_date'
  | 'msg_presence_long_ago';

export interface PresenceText {
  key: PresenceKey;
  params?: Record<string, string>;
}

const MINUTE = 60;
const HOUR = 3600;
const DAY = 86_400;

/** `6 Oct`: a short date in the app's language. */
export function shortDate(unix: number): string {
  return new Date(unix * 1000).toLocaleDateString(lang(), { day: 'numeric', month: 'short' });
}

const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

/** Whether `seenAt` falls on the calendar day before `now`, in local time. */
function isYesterday(seenAt: number, now: number): boolean {
  const y = new Date(now * 1000);
  y.setDate(y.getDate() - 1);
  return sameDay(new Date(seenAt * 1000), y);
}

/**
 * Online while the last heartbeat holds (`now < onlineUntil`); after that, by
 * how long ago it was. The hours win over "yesterday": a contact seen at
 * 23:00 is "1 h ago" at half past midnight. Times are unix seconds.
 */
export function presenceKey(seenAt: number, onlineUntil: number, now: number, fmt: (unix: number) => string = shortDate): PresenceText {
  if (now < onlineUntil) return { key: 'msg_presence_online' };
  // A clock behind the heartbeat's makes the age negative: it was just now.
  const age = Math.max(0, now - seenAt);
  if (age < MINUTE) return { key: 'msg_presence_just_now' };
  if (age < HOUR) return { key: 'msg_presence_minutes', params: { n: String(Math.floor(age / MINUTE)) } };
  if (age < DAY) return { key: 'msg_presence_hours', params: { n: String(Math.floor(age / HOUR)) } };
  if (isYesterday(seenAt, now)) return { key: 'msg_presence_yesterday' };
  if (age < 30 * DAY) return { key: 'msg_presence_date', params: { date: fmt(seenAt) } };
  return { key: 'msg_presence_long_ago' };
}

/** The same, worded by `t`. */
export function presenceLabel(
  seenAt: number,
  onlineUntil: number,
  now: number,
  t: (key: PresenceKey, vars?: Record<string, string>) => string,
  fmt?: (unix: number) => string,
): string {
  const { key, params } = presenceKey(seenAt, onlineUntil, now, fmt);
  return t(key, params);
}

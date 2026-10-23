// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/moderation/log.ts at 8ce09c39, then changed for channels.

// The log of moderation of a channel as the screen words it: the action by the name
// the core writes, and what the core says of a line in JSON (a reason,
// a span, whether a mark is still in force, the settings set) as words,
// never as the JSON itself. Plain functions, so the tests pin every shape.

import type { LogEntry, ScheduleView as Schedule } from '../api';
import { scheduleText } from './schedule';

export type Tr = (key: string, params?: Record<string, string>) => string;

/** The actions the core writes (`LogLine.action`); any other is shown as its word. */
export const KNOWN = new Set(['hide', 'unhide', 'approve', 'reject', 'ban', 'unban', 'mute', 'unmute', 'trust', 'untrust', 'remove', 'pin', 'unpin', 'settings']);
/** These name a person; the others a message (or, for the hiding of a span, its author). */
export const ABOUT_PEOPLE = new Set(['ban', 'unban', 'mute', 'unmute', 'trust', 'untrust', 'remove']);

const JOINS = ['open', 'request', 'invite'];
const HISTORIES = ['open', 'from_join'];
const PREMODS = ['off', 'newcomers', 'all'];

/** What the core says of a line, by the kind of line (`LogLine.detail`). */
export interface Detail {
  active?: boolean;
  note?: string;
  reason?: string;
  from?: number;
  to?: number;
  version?: string;
  until?: number | null;
  post_mode?: string;
  premod?: string;
  schedule?: Schedule | null;
  comments?: string;
  join?: string;
  history?: string;
}

/** The detail as the core sent it; nothing for an empty one or one of another shape. */
export function parseDetail(detail: string | undefined): Detail | null {
  if (!detail) return null;
  try {
    const v: unknown = JSON.parse(detail);
    return v && typeof v === 'object' && !Array.isArray(v) ? (v as Detail) : null;
  } catch {
    return null;
  }
}

/** Whether the line is about a person: by its action, or a hiding of everything an author wrote in a span. */
export function aboutPerson(e: LogEntry): boolean {
  return ABOUT_PEOPLE.has(e.action) || typeof parseDetail(e.detail)?.to === 'number';
}

/** The name of the action. */
export function actionText(e: LogEntry, tr: Tr): string {
  return KNOWN.has(e.action) ? tr(`chan_mod_log_${e.action}`) : e.action;
}

/**
 * What else the line says, in words; empty when nothing. `stamp` words a
 * time on this device's clock, `names` are the weekdays for a schedule.
 */
export function detailText(e: LogEntry, tr: Tr, stamp: (unix: number) => string, names: string[]): string {
  const d = parseDetail(e.detail);
  if (!d) return '';
  const parts: string[] = [];
  switch (e.action) {
    case 'hide':
    case 'unhide':
    case 'approve':
    case 'reject':
      if (d.reason === 'spam') parts.push(tr('chan_mod_spam'));
      if (typeof d.to === 'number') {
        parts.push(d.from ? tr('chan_mod_log_span', { from: stamp(d.from), to: stamp(d.to) }) : tr('chan_mod_log_span_all', { to: stamp(d.to) }));
      }
      if (d.active === false) parts.push(tr('chan_mod_log_inactive'));
      break;
    case 'mute':
      parts.push(typeof d.until === 'number' ? tr('chan_mod_log_until', { time: stamp(d.until) }) : tr('chan_mod_mute_span_forever'));
      break;
    case 'settings': {
      const word = (prefix: string, known: string[], v: string) => (known.includes(v) ? tr(`${prefix}${v}`) : v);
      if (d.join) parts.push(`${tr('chan_rule_join')}: ${word('chan_join_', JOINS, d.join)}`);
      if (d.history) parts.push(`${tr('chan_rule_history')}: ${word('chan_history_', HISTORIES, d.history)}`);
      // `post_mode` of an older build is not said: posts are by authors and the team, whatever it named (2026-10-22).
      if (d.comments) parts.push(`${tr('chan_rule_comments')}: ${d.comments === 'on' ? tr('chan_on') : d.comments === 'off' ? tr('chan_off') : d.comments}`);
      if (d.premod) parts.push(`${tr('chan_rule_premod')}: ${word('chan_premod_', PREMODS, d.premod)}`);
      if ('schedule' in d) parts.push(`${tr('chan_rule_schedule')}: ${d.schedule?.days.length ? scheduleText(d.schedule, names) : tr('chan_schedule_off')}`);
      break;
    }
  }
  return parts.join(' · ');
}

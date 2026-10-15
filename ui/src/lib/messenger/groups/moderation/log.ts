// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The log of moderation as the screen words it: the action by the name
// the core writes, and what the core says of a line in JSON (a reason,
// a span, whether a mark is still in force, the settings set) as words,
// never as the JSON itself. Plain functions, so the tests pin every shape.

import type { LogEntry, Schedule } from '../../api';
import { scheduleText } from './schedule';

export type Tr = (key: string, params?: Record<string, string>) => string;

/** The actions the core writes (`LogLine.action`); any other is shown as its word. */
export const KNOWN = new Set(['hide', 'unhide', 'approve', 'reject', 'ban', 'unban', 'mute', 'unmute', 'trust', 'untrust', 'settings']);
/** These name a person; the others a message (or, for the hiding of a span, its author). */
export const ABOUT_PEOPLE = new Set(['ban', 'unban', 'mute', 'unmute', 'trust', 'untrust']);

const POST_MODES = ['all', 'managers'];
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
  return KNOWN.has(e.action) ? tr(`msg_mod_log_${e.action}`) : e.action;
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
      if (d.reason === 'spam') parts.push(tr('msg_mod_spam'));
      if (typeof d.to === 'number') {
        parts.push(d.from ? tr('msg_mod_log_span', { from: stamp(d.from), to: stamp(d.to) }) : tr('msg_mod_log_span_all', { to: stamp(d.to) }));
      }
      if (d.active === false) parts.push(tr('msg_mod_log_inactive'));
      break;
    case 'mute':
      parts.push(typeof d.until === 'number' ? tr('msg_mod_log_until', { time: stamp(d.until) }) : tr('msg_mod_mute_span_forever'));
      break;
    case 'settings':
      if (d.post_mode) parts.push(`${tr('msg_mod_post_mode')}: ${POST_MODES.includes(d.post_mode) ? tr(`msg_mod_post_mode_${d.post_mode}`) : d.post_mode}`);
      if (d.premod) parts.push(`${tr('msg_mod_premod')}: ${PREMODS.includes(d.premod) ? tr(`msg_mod_premod_${d.premod}`) : d.premod}`);
      if ('schedule' in d) parts.push(`${tr('msg_mod_schedule')}: ${d.schedule ? scheduleText(d.schedule, names) : tr('msg_mod_schedule_off')}`);
      break;
  }
  return parts.join(' · ');
}

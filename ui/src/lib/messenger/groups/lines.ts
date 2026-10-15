// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What happened to a group, as a line in its chat.

import type { MessengerMessage, Schedule } from '../api';
import { get } from 'svelte/store';
import { localeTag, locale } from '$lib/core/i18n';
import { stamp } from '../shared/time';
import { scheduleText, weekdayNames } from './moderation/schedule';

type Translate = (key: string, params?: Record<string, string>) => string;

/** How a line words a time and the days of a week; the app's clock and language unless a test says otherwise. */
export interface Words {
  stamp: (unix: number) => string;
  days: () => string[];
}

const appWords: Words = { stamp, days: () => weekdayNames(localeTag(get(locale))) };

const KNOWN = new Set([
  'group_created', 'group_admitted', 'group_joined', 'group_left', 'group_removed', 'group_banned', 'group_unbanned',
  'group_role', 'group_muted', 'group_unmuted', 'group_edited', 'group_owner', 'group_link_changed', 'group_disbanded',
  'group_muted_until', 'group_trusted', 'group_untrusted', 'group_post_mode', 'group_premod', 'group_schedule',
]);

// The words of the settings screen for the two rules with a fixed set of values.
const POST_MODES = ['all', 'managers'];
const PREMODS = ['off', 'newcomers', 'all'];

interface Details {
  actor?: string;
  target?: string | null;
  role?: string | null;
  until?: number;
  post_mode?: string;
  premod?: string;
  schedule?: Schedule | null;
}

/** A value of a rule as the settings screen words it; any other value (a newer core) as it came. */
const choice = (tr: Translate, prefix: string, known: string[], v: string | undefined): string =>
  v === undefined ? '' : known.includes(v) ? tr(`${prefix}${v}`) : v;

/** `null` when the line is not about a group. */
export function groupLine(
  m: MessengerMessage,
  tr: Translate,
  name: (pubkey: string) => string,
  words: Words = appWords,
): string | null {
  if (m.content_type !== 'system' || !m.text || !KNOWN.has(m.text)) return null;
  const d = (m.media ?? {}) as Details;
  const actor = d.actor ? name(d.actor) : '';
  const target = d.target ? name(d.target) : '';
  const role = d.role ? tr(`msg_group_role_${d.role}`) : '';
  const time = typeof d.until === 'number' ? words.stamp(d.until) : '';
  // The value of the rule the line names: who may write, or whom the check covers.
  const mode = m.text === 'group_premod' ? choice(tr, 'msg_mod_premod_', PREMODS, d.premod) : choice(tr, 'msg_mod_post_mode_', POST_MODES, d.post_mode);
  const schedule = m.text !== 'group_schedule' ? '' : d.schedule ? `${scheduleText(d.schedule, words.days())} (${d.schedule.tz})` : tr('msg_mod_schedule_off');
  return tr(`msg_sys_${m.text}`, { actor, target, role, time, mode, schedule });
}

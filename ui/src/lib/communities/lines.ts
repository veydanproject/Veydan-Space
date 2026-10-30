// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/lines.ts at 8ce09c39, then changed for channels.

// What a channel says of itself in one line: its row in the list of chats
// (the newest post, or where I stand in it) and the rules on its card (who
// gets in, who posts, the comments, the check, the hours). Plain functions,
// so the tests pin every shape.

import type { ChannelDetail, ChannelView } from './api';
import { scheduleText } from './moderation/schedule';

type Translate = (key: string, params?: Record<string, string>) => string;

export const JOINS = ['open', 'request', 'invite'];
export const HISTORIES = ['open', 'from_join'];
const PREMODS = ['off', 'newcomers', 'all'];

/** A value of a rule as the settings word it; any other value (a newer core) as it came. */
export const choice = (tr: Translate, prefix: string, known: string[], v: string): string => (known.includes(v) ? tr(`${prefix}${v}`) : v);

const OUT_REASONS = ['left', 'removed', 'banned'];

/** The key of «out of the channel», by why (`out_reason`): left, removed, banned; a newer reason as plain «out». */
export function outKey(c: Pick<ChannelView, 'out_reason'>, prefix: 'chan_state_out' | 'chan_out_text'): string {
  return c.out_reason && OUT_REASONS.includes(c.out_reason) ? `${prefix}_${c.out_reason}` : prefix;
}

/** The line under the name in the list of chats. */
export function previewLine(c: ChannelView, tr: Translate): string {
  switch (c.membership) {
    case 'joining': return tr('chan_state_joining');
    case 'requested': return tr('chan_state_requested');
    case 'rejected': return tr('chan_state_rejected');
    case 'out': return tr(outKey(c, 'chan_state_out'));
    case 'disbanded': return tr('chan_state_disbanded');
    case 'none': return tr('chan_state_none');
  }
  const last = c.last;
  if (!last) return tr('chan_no_posts');
  if (last.text?.trim()) return last.text.trim().split('\n')[0];
  // A voice message or a circle by what it is, in the messenger's words for a chat's row.
  if (last.media_kind === 'voice' || last.media_kind === 'circle') return tr(`msg_notice_${last.media_kind}`);
  return last.has_media ? tr('chan_post_file') : tr('chan_post_gone');
}

/** How one gets in: the word of the join and of the history together. */
export function joinLine(d: Pick<ChannelDetail, 'history'> & { channel: Pick<ChannelView, 'join'> }, tr: Translate): string {
  const join = choice(tr, 'chan_join_', JOINS, d.channel.join);
  return `${join} · ${choice(tr, 'chan_history_', HISTORIES, d.history)}`;
}

/** The rules on the card, one line each; `days` are the names of the weekdays, Monday first. */
export function rulesLines(d: ChannelDetail, tr: Translate, days: string[]): string[] {
  const lines = [
    joinLine(d, tr),
    `${tr('chan_rule_comments')}: ${d.comments === 'on' ? tr('chan_on') : d.comments === 'off' ? tr('chan_off') : d.comments}`,
  ];
  if (d.premod !== 'off') lines.push(`${tr('chan_rule_premod')}: ${choice(tr, 'chan_premod_', PREMODS, d.premod)}`);
  if (d.schedule?.days.length) lines.push(`${tr('chan_rule_schedule')}: ${scheduleText(d.schedule, days)} (${d.schedule.tz})`);
  return lines;
}

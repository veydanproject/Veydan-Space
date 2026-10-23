// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/moderation/log.test.ts at 8ce09c39, then changed for channels.

import { describe, expect, it } from 'vitest';
import type { LogEntry } from '../api';
import { aboutPerson, actionText, detailText, parseDetail } from './log';

const tr = (key: string, params?: Record<string, string>) => (params ? `${key}(${Object.values(params).join(',')})` : key);
const stamp = (unix: number) => `t${unix}`;
const names = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const line = (action: string, detail = '', target = 'x'): LogEntry => ({ at: 1, by: 'b', action, target, detail });

describe('parseDetail', () => {
  it('reads the JSON of the core and nothing else', () => {
    expect(parseDetail('')).toBeNull();
    expect(parseDetail(undefined)).toBeNull();
    expect(parseDetail('spam')).toBeNull();
    expect(parseDetail('[1]')).toBeNull();
    expect(parseDetail('{"active":false,"note":"n"}')).toEqual({ active: false, note: 'n' });
  });
});

describe('actionText and aboutPerson', () => {
  it('names the actions the core writes and shows any other as its word', () => {
    for (const a of ['hide', 'unhide', 'approve', 'reject', 'ban', 'unban', 'mute', 'unmute', 'trust', 'untrust', 'remove', 'pin', 'unpin', 'settings']) {
      expect(actionText(line(a), tr)).toBe(`chan_mod_log_${a}`);
    }
    expect(actionText(line('set_muted'), tr)).toBe('set_muted');
  });

  it('tells a line about a person from one about a message', () => {
    for (const a of ['ban', 'unban', 'mute', 'unmute', 'trust', 'untrust', 'remove']) expect(aboutPerson(line(a))).toBe(true);
    expect(aboutPerson(line('hide', '{"active":true,"note":"n"}'))).toBe(false);
    expect(aboutPerson(line('hide', '{"active":true,"note":"n","from":0,"to":100,"reason":"spam"}'))).toBe(true);
    expect(aboutPerson(line('settings', '{"premod":"all"}', ''))).toBe(false);
  });
});

describe('detailText', () => {
  it('words a mark: the reason, the span, and whether it still holds', () => {
    expect(detailText(line('hide', '{"active":true,"note":"n"}'), tr, stamp, names)).toBe('');
    expect(detailText(line('hide', '{"active":true,"note":"n","reason":"spam","from":0,"to":100}'), tr, stamp, names)).toBe('chan_mod_spam · chan_mod_log_span_all(t100)');
    expect(detailText(line('hide', '{"active":false,"note":"n","from":50,"to":100}'), tr, stamp, names)).toBe('chan_mod_log_span(t50,t100) · chan_mod_log_inactive');
    expect(detailText(line('approve', '{"active":true,"note":"n","version":"v"}'), tr, stamp, names)).toBe('');
    expect(detailText(line('reject', '{"active":false,"note":"n"}'), tr, stamp, names)).toBe('chan_mod_log_inactive');
  });

  it('words a mute by its end and the settings by their names', () => {
    expect(detailText(line('mute', '{"until":3600}'), tr, stamp, names)).toBe('chan_mod_log_until(t3600)');
    expect(detailText(line('mute', '{"until":null}'), tr, stamp, names)).toBe('chan_mod_mute_span_forever');
    expect(detailText(line('unmute', '{}'), tr, stamp, names)).toBe('');
    expect(detailText(line('ban', '{}'), tr, stamp, names)).toBe('');
    expect(detailText(line('settings', '{"post_mode":"authors","premod":"newcomers"}', ''), tr, stamp, names)).toBe(
      'chan_rule_premod: chan_premod_newcomers',
    );
    expect(detailText(line('settings', '{"join":"request","history":"from_join","comments":"off"}', ''), tr, stamp, names)).toBe(
      'chan_rule_join: chan_join_request · chan_rule_history: chan_history_from_join · chan_rule_comments: chan_off',
    );
    expect(detailText(line('settings', '{"schedule":null}', ''), tr, stamp, names)).toBe('chan_rule_schedule: chan_schedule_off');
    expect(detailText(line('settings', '{"schedule":{"tz":"UTC","days":[1,2,3,4,5],"from":"09:00","to":"18:00"}}', ''), tr, stamp, names)).toBe(
      'chan_rule_schedule: Mon–Fri 09:00–18:00',
    );
    expect(detailText(line('settings', '{"post_mode":"all"}', ''), tr, stamp, names)).toBe('');
  });

  it('says nothing of a detail it cannot read', () => {
    expect(detailText(line('hide', 'spam'), tr, stamp, names)).toBe('');
    expect(detailText(line('hide', ''), tr, stamp, names)).toBe('');
  });
});

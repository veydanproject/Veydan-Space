// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { presenceKey, presenceLabel } from './labels';

/** Unix seconds of a local wall-clock time. */
const at = (y: number, mo: number, d: number, h = 0, mi = 0, s = 0) => Math.floor(new Date(y, mo - 1, d, h, mi, s).getTime() / 1000);
const fmt = (unix: number) => `date:${unix}`;

describe('presenceKey', () => {
  const now = at(2026, 10, 6, 12, 0, 0);

  it('is online while the heartbeat holds, whatever its age', () => {
    expect(presenceKey(now - 5, now + 1, now).key).toBe('msg_presence_online');
    expect(presenceKey(now - 5, now, now).key).toBe('msg_presence_just_now');
  });

  it('says "just now" up to 59 s and minutes from 60 s', () => {
    expect(presenceKey(now - 59, now - 1, now)).toEqual({ key: 'msg_presence_just_now' });
    expect(presenceKey(now - 60, now - 1, now)).toEqual({ key: 'msg_presence_minutes', params: { n: '1' } });
    expect(presenceKey(now - 3599, now - 1, now)).toEqual({ key: 'msg_presence_minutes', params: { n: '59' } });
  });

  it('counts hours up to 23:59', () => {
    expect(presenceKey(now - 3600, now - 1, now)).toEqual({ key: 'msg_presence_hours', params: { n: '1' } });
    const h2359 = 23 * 3600 + 59 * 60;
    expect(presenceKey(now - h2359, now - 1, now)).toEqual({ key: 'msg_presence_hours', params: { n: '23' } });
  });

  it('keeps hours across midnight, then says yesterday for the day before', () => {
    const night = at(2026, 10, 6, 0, 30);
    expect(presenceKey(at(2026, 10, 5, 23, 0), 0, night)).toEqual({ key: 'msg_presence_hours', params: { n: '1' } });
    // 25 h ago is the calendar day before.
    expect(presenceKey(at(2026, 10, 5, 11, 0), 0, now)).toEqual({ key: 'msg_presence_yesterday' });
    // 25.5 h ago at 00:30 is two calendar days before: a date.
    expect(presenceKey(at(2026, 10, 4, 23, 0), 0, night, fmt)).toEqual({ key: 'msg_presence_date', params: { date: fmt(at(2026, 10, 4, 23, 0)) } });
  });

  it('shows a date within 30 days and "a long time ago" after', () => {
    const seen = now - 29 * 86_400;
    expect(presenceKey(seen, 0, now, fmt)).toEqual({ key: 'msg_presence_date', params: { date: fmt(seen) } });
    expect(presenceKey(now - 30 * 86_400, 0, now, fmt)).toEqual({ key: 'msg_presence_long_ago' });
  });

  it('treats a heartbeat from the future as just now', () => {
    expect(presenceKey(now + 30, now - 1, now)).toEqual({ key: 'msg_presence_just_now' });
  });
});

describe('presenceLabel', () => {
  it('words the key with its params', () => {
    const t = (key: string, vars?: Record<string, string>) => `${key}${vars ? JSON.stringify(vars) : ''}`;
    expect(presenceLabel(100, 0, 100 + 125, t)).toBe('msg_presence_minutes{"n":"2"}');
    expect(presenceLabel(100, 300, 200, t)).toBe('msg_presence_online');
  });
});

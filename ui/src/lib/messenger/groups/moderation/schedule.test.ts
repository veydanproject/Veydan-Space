// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import {
  daysText, hideFrom, muteUntil, opensAt, periodSince, scheduleText, scheduleValid, validTime, weekdayNames, zoneHint, zoneList,
} from './schedule';

const EN = weekdayNames('en');

describe('weekdayNames', () => {
  it('starts on Monday', () => {
    expect(EN).toEqual(['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']);
    expect(weekdayNames('ru')[0].toLowerCase()).toBe('пн');
  });
});

describe('daysText and scheduleText', () => {
  it('makes a span of three days and more, lists the rest', () => {
    expect(daysText([1, 2, 3, 4, 5], EN)).toBe('Mon–Fri');
    expect(daysText([1, 3, 5], EN)).toBe('Mon, Wed, Fri');
    expect(daysText([6, 7], EN)).toBe('Sat, Sun');
    expect(daysText([5, 1, 2, 3, 7], EN)).toBe('Mon–Wed, Fri, Sun');
    expect(daysText([], EN)).toBe('');
  });

  it('leaves the days out when it is every day, and keeps a night as it is', () => {
    expect(scheduleText({ tz: 'UTC', days: [1, 2, 3, 4, 5], from: '09:00', to: '18:00' }, EN)).toBe('Mon–Fri 09:00–18:00');
    expect(scheduleText({ tz: 'UTC', days: [1, 2, 3, 4, 5, 6, 7], from: '22:00', to: '06:00' }, EN)).toBe('22:00–06:00');
  });
});

describe('scheduleValid', () => {
  const ok = { tz: 'Europe/Berlin', days: [1], from: '09:00', to: '18:00' };
  it('takes a day and two different real times', () => {
    expect(scheduleValid(ok)).toBe(true);
    expect(scheduleValid({ ...ok, from: '22:00', to: '06:00' })).toBe(true);
  });
  it('refuses no day, an empty window, a time that is not one, a day out of the week', () => {
    expect(scheduleValid({ ...ok, days: [] })).toBe(false);
    expect(scheduleValid({ ...ok, to: '09:00' })).toBe(false);
    expect(scheduleValid({ ...ok, to: '24:00' })).toBe(false);
    expect(scheduleValid({ ...ok, days: [0] })).toBe(false);
    expect(validTime('7:00')).toBe(false);
  });
});

describe('opensAt', () => {
  // Thursday 15 October 2026, 12:00 UTC.
  const now = Date.UTC(2026, 9, 15, 12, 0) / 1000;

  it('says the time on the group clock, and the day when it is not today there', () => {
    expect(opensAt(now + 2 * 3600, now, 'ru', 'UTC')).toEqual({ time: '14:00', day: null });
    expect(opensAt(now + 2 * 3600, now, 'ru', 'Europe/Berlin')).toEqual({ time: '16:00', day: null });
    const next = opensAt(now + 20 * 3600, now, 'en', 'UTC');
    expect(next?.day).toBe('Fri');
    // 08:00 in Moscow is already Friday there, though the device in UTC still has Thursday.
    expect(opensAt(Date.UTC(2026, 9, 16, 5, 0) / 1000, now, 'ru', 'Europe/Moscow')?.time).toBe('08:00');
  });

  it('says nothing of no time, a time gone, or one too far to be the opening of the hours', () => {
    expect(opensAt(null, now, 'en', 'UTC')).toBeNull();
    expect(opensAt(now - 1, now, 'en', 'UTC')).toBeNull();
    expect(opensAt(now + 30 * 86400, now, 'en', 'UTC')).toBeNull();
  });

  it('falls back to the device clock for a zone it does not know', () => {
    expect(opensAt(now + 3600, now, 'en', 'Mars/Olympus')).not.toBeNull();
  });
});

describe('the spans of the choices', () => {
  const now = 1_000_000;
  it('ends a mute after an hour, a day, a week, or never', () => {
    expect(muteUntil('hour', now)).toBe(now + 3600);
    expect(muteUntil('day', now)).toBe(now + 86400);
    expect(muteUntil('week', now)).toBe(now + 7 * 86400);
    expect(muteUntil('forever', now)).toBeUndefined();
  });

  it('reaches back an hour, a day, or to the beginning', () => {
    expect(hideFrom('hour', now)).toBe(now - 3600);
    expect(hideFrom('day', now)).toBe(now - 86400);
    expect(hideFrom('all', now)).toBe(0);
  });

  it('starts today at the local midnight and a week seven days back', () => {
    const at = new Date(2026, 9, 15, 13, 30);
    expect(periodSince('today', at)).toBe(new Date(2026, 9, 15).getTime() / 1000);
    expect(periodSince('week', at)).toBe(Math.floor(at.getTime() / 1000) - 7 * 86400);
    expect(periodSince('all', at)).toBeUndefined();
  });
});

describe('zoneHint', () => {
  it('names the zone the core does not know, and nothing otherwise', () => {
    const s = { tz: 'Mars/Olympus', days: [1], from: '09:00', to: '18:00' };
    expect(zoneHint({ schedule: s, schedule_tz_unknown: true })).toBe('Mars/Olympus');
    expect(zoneHint({ schedule: s, schedule_tz_unknown: false })).toBeNull();
    expect(zoneHint({ schedule: s })).toBeNull();
    expect(zoneHint({ schedule: null, schedule_tz_unknown: true })).toBeNull();
  });
});

describe('zoneList', () => {
  it('holds the zones asked for and UTC, sorted, once each', () => {
    const list = zoneList('Europe/Berlin', 'Europe/Berlin');
    expect(list).toContain('UTC');
    expect(list.filter((z) => z === 'Europe/Berlin')).toHaveLength(1);
    expect([...list].sort()).toEqual(list);
  });
});

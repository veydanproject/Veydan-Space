// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { CallLogEntry } from '../generated/calls';
import { foldLog, isLogMissed, lineOf, logSections } from './log';

const DAY = 86_400;
/** Days by whole days of unix time: the test does not depend on the zone. */
const dayOf = (unix: number) => String(Math.floor(unix / DAY));

const entry = (id: string, over: Partial<CallLogEntry> = {}): CallLogEntry => ({
  call_id: id,
  chat_id: 'dm:aa',
  peer: 'aa',
  kind: 'dm',
  direction: 'in',
  media: 'audio',
  started_at: 10 * DAY + 100,
  answered_at: null,
  ended_at: null,
  outcome: 'missed',
  duration_secs: null,
  via_relay: false,
  started_by: null,
  participants: 0,
  ...over,
});

describe('the journal of calls', () => {
  it('calls missed only what rang here between two', () => {
    expect(isLogMissed(entry('a'))).toBe(true);
    expect(isLogMissed(entry('b', { outcome: 'busy' }))).toBe(true);
    expect(isLogMissed(entry('c', { direction: 'out' }))).toBe(false);
    expect(isLogMissed(entry('d', { outcome: 'ended', answered_at: 1, duration_secs: 60 }))).toBe(false);
    expect(isLogMissed(entry('g', { kind: 'group', chat_id: 'group:gg', outcome: 'missed' }))).toBe(false);
  });

  it('gives the line of the chat its facts: the way of a call that was talked', () => {
    expect(lineOf(entry('a', { outcome: 'ended', via_relay: true, duration_secs: 30 }))).toMatchObject({ outcome: 'ended', via: 'relay', duration: 30 });
    expect(lineOf(entry('b')).via).toBeNull();
  });

  it('folds neighbours of one chat, day, way and kind into one row', () => {
    const t = 10 * DAY + 5000;
    const rows = foldLog([
      entry('a3', { started_at: t }),
      entry('a2', { started_at: t - 10 }),
      entry('a1', { started_at: t - 20 }),
      entry('b', { started_at: t - 30, chat_id: 'dm:bb', peer: 'bb' }),
      entry('a0', { started_at: t - 40 }),
      entry('out', { started_at: t - 50, direction: 'out', outcome: 'ended', duration_secs: 5 }),
      entry('old', { started_at: t - 2 * DAY }),
    ], dayOf);
    expect(rows.map((r) => [r.head.call_id, r.calls.length])).toEqual([['a3', 3], ['b', 1], ['a0', 1], ['out', 1], ['old', 1]]);
    expect(rows[0].missed).toBe(true);
    expect(rows[3].missed).toBe(false);
    expect(logSections(rows, dayOf).map((s) => s.rows.length)).toEqual([4, 1]);
  });
});

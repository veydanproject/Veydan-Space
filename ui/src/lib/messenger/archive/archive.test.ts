// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { etaSeconds, monthLabel, percentOf, stateOf, yearsLabel } from './archive';
import { IDLE_PROGRESS, type ArchiveProgress } from './types';

const restoring = (done: number, total: number): ArchiveProgress => ({ phase: 'restoring', done: 0, total: 10, bytes_done: done, bytes_total: total });

describe('percentOf', () => {
  it('counts bytes, and segments while the bytes are not known', () => {
    expect(percentOf(restoring(630, 1000))).toBe(63);
    expect(percentOf({ ...restoring(0, 0), done: 3, total: 10 })).toBe(30);
    expect(percentOf(IDLE_PROGRESS)).toBe(0);
  });
  it('is never full while segments still come', () => {
    expect(percentOf(restoring(1000, 1000))).toBe(99);
    expect(percentOf({ ...restoring(1000, 1000), phase: 'writing' })).toBe(100);
  });
});

describe('etaSeconds', () => {
  it('goes by the rate since the first sample', () => {
    expect(etaSeconds(restoring(500, 1000), { at: 0, bytes: 0 }, 5000)).toBe(5);
  });
  it('says nothing before it can tell', () => {
    expect(etaSeconds(restoring(500, 1000), null, 5000)).toBeNull();
    expect(etaSeconds(restoring(0, 1000), { at: 0, bytes: 0 }, 5000)).toBeNull();
    expect(etaSeconds(restoring(10, 1000), { at: 0, bytes: 0 }, 500)).toBeNull();
  });
});

describe('labels', () => {
  it('names the years of the archive', () => {
    expect(yearsLabel('2024-03', '2025-10')).toBe('2024–2025');
    expect(yearsLabel('2026-01', '2026-10')).toBe('2026');
    expect(yearsLabel(null, null)).toBeNull();
  });
  it('names a month without the mark of the year', () => {
    expect(monthLabel('2026-10', 'ru')).toBe('октябрь 2026');
    expect(monthLabel('nope', 'ru')).toBeNull();
  });
});

describe('stateOf', () => {
  const none = IDLE_PROGRESS;
  it('tells why the archive stands', () => {
    expect(stateOf(true, none, { why: 'read_only', until: 5 })).toBe('paused_read_only_until');
    expect(stateOf(true, none, { why: 'read_only' })).toBe('paused_read_only');
    expect(stateOf(true, none, { why: 'quota' })).toBe('paused_quota');
    expect(stateOf(true, none, { why: 'offline' })).toBe('paused_offline');
    expect(stateOf(true, none, { why: 'no_storage' })).toBe('unavailable');
    expect(stateOf(false, none, null)).toBe('unavailable');
  });
  it('reads the reason of a paused progress too', () => {
    expect(stateOf(true, { ...none, phase: 'paused', reason: { why: 'quota' } }, null)).toBe('paused_quota');
  });
  it('tells the work', () => {
    expect(stateOf(true, restoring(1, 2), null)).toBe('restoring');
    expect(stateOf(true, { ...none, phase: 'writing' }, null)).toBe('writing');
    expect(stateOf(true, none, null)).toBe('idle');
  });
});

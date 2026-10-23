// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { KeeperStatus } from './api';
import { TAKE_STALL_SECS, keeperBackground, keeperMove } from './keeper';

const status = (over: Partial<KeeperStatus> = {}): KeeperStatus => ({
  kept_here: true, online: true, readers: 0, generations: 1, garbage: 0, rotate_asked: false,
  background: false, keep_on: false, background_running: false, owner: false, handing: false, has_root: false, ...over,
});

describe('the keeper in the background (Android, 2026-10-22)', () => {
  it('shows the switch and its state on a phone that keeps the channel in the background', () => {
    expect(keeperBackground(status({ background: true }), true)).toEqual({ switch: true, keepOn: false, running: false, asleep: false });
    expect(keeperBackground(status({ background: true, keep_on: true, background_running: true }), true))
      .toEqual({ switch: true, keepOn: true, running: true, asleep: false });
  });

  it('hides the switch on a desktop, and keeps the warning on a phone without the background', () => {
    expect(keeperBackground(status(), false)).toEqual({ switch: false, keepOn: false, running: false, asleep: false });
    expect(keeperBackground(status(), true)).toEqual({ switch: false, keepOn: false, running: false, asleep: true });
  });

  it('shows nothing for a channel kept elsewhere, whatever is said of the service', () => {
    const elsewhere = status({ kept_here: false, background: true, keep_on: true, background_running: true });
    expect(keeperBackground(elsewhere, true)).toEqual({ switch: false, keepOn: false, running: false, asleep: false });
    expect(keeperBackground(null, true)).toEqual({ switch: false, keepOn: false, running: false, asleep: false });
  });
});

describe('the keeper between the owner’s devices (wire §15, 2026-10-23)', () => {
  const now = 1_800_000_000;
  it('offers the owner on another device to take the keeper over, and a reader nothing', () => {
    expect(keeperMove(status({ kept_here: false, owner: true }), true, now)).toEqual({ place: 'take', emergency: false });
    expect(keeperMove(status({ kept_here: false }), true, now)).toEqual({ place: 'elsewhere', emergency: false });
    expect(keeperMove(null, false, now)).toEqual({ place: 'unknown', emergency: false });
    expect(keeperMove(status(), true, now)).toEqual({ place: 'here', emergency: false });
  });

  it('says «Передаём…» on both sides, and offers the recovery only after a long wait with the root here', () => {
    expect(keeperMove(status({ kept_here: false, owner: true, handing: true }), true, now).place).toBe('handing');
    const taking = status({ kept_here: false, owner: true, taking: 'handover', taking_since: now - 60, has_root: true });
    expect(keeperMove(taking, true, now)).toEqual({ place: 'taking', emergency: false });
    expect(keeperMove({ ...taking, taking_since: now - TAKE_STALL_SECS }, true, now)).toEqual({ place: 'taking', emergency: true });
    expect(keeperMove({ ...taking, taking_since: now - TAKE_STALL_SECS, has_root: false }, true, now).emergency).toBe(false);
    expect(keeperMove({ ...taking, taking: 'emergency', taking_since: now - TAKE_STALL_SECS }, true, now).emergency).toBe(false);
  });
});

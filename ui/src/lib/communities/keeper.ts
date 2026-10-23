// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The keeper block of ChannelInfo on a phone that runs the keeper in the
// background (Android, internal/communities-stage1.md §4, 2026-10-22):
// the owner's switch «Держать хранителя включённым» and the state
// «работает в фоне». A desktop and a phone without it see neither; a phone
// without it keeps the warning that new readers wait for the app.

import type { KeeperStatus } from './api';

export interface KeeperBackground {
  /** The switch is shown: this device keeps the channel and can run it in the background. */
  switch: boolean;
  /** The switch is on. */
  keepOn: boolean;
  /** «работает в фоне»: the keeper's service of the phone runs now. */
  running: boolean;
  /** The warning that new readers wait until the app is opened. */
  asleep: boolean;
}

export function keeperBackground(k: KeeperStatus | null, phone: boolean): KeeperBackground {
  const here = !!k?.kept_here;
  const background = here && !!k?.background;
  return {
    switch: background,
    keepOn: background && !!k?.keep_on,
    running: background && !!k?.background_running,
    asleep: here && phone && !background,
  };
}

/**
 * How long a device waits for the other device of the owner to hand the
 * keeper over before it offers to recover it here instead (wire §15).
 */
export const TAKE_STALL_SECS = 10 * 60;

/**
 * Where the keeper is, for the line of the block and its buttons (wire §15,
 * 2026-10-23): `here`; `handing` (this device hands it to the owner's other
 * one); `taking` (this device waits for it); `take` (the owner on a device
 * that does not keep it: «Сделать хранителем это устройство»); `elsewhere`
 * (a reader: another device keeps it); `unknown`.
 */
export type KeeperPlace = 'here' | 'handing' | 'taking' | 'take' | 'elsewhere' | 'unknown';

export interface KeeperMove {
  place: KeeperPlace;
  /** «Другое устройство потеряно — восстановить здесь»: the wait went on too long, and the root is here. */
  emergency: boolean;
}

export function keeperMove(k: KeeperStatus | null, known: boolean, now: number): KeeperMove {
  if (k?.handing) return { place: 'handing', emergency: false };
  if (k?.kept_here) return { place: 'here', emergency: false };
  if (k?.taking) {
    const stalled = k.taking === 'handover' && now - (k.taking_since ?? now) >= TAKE_STALL_SECS;
    return { place: 'taking', emergency: stalled && !!k.has_root };
  }
  if (k?.owner) return { place: 'take', emergency: false };
  return { place: known ? 'elsewhere' : 'unknown', emergency: false };
}

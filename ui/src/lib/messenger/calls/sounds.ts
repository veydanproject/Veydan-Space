// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Which sound a call makes when it connects and when it ends. Pure, so the
// choice is tested without Web Audio (CallSounds plays it, tones.ts makes it).

import type { CallView } from '../api';
import type { EndedCall } from './callStore.svelte';

/** How a call that ended sounds: the falling chime, the "no" of a refusal, or nothing. */
export type EndSound = 'end' | 'busy' | null;

/**
 * A talk that ended, or a call I gave up: the falling chime. A call of mine
 * the peer declined, was busy for, never answered, or that could not
 * connect: the "no". A call to me that never talked here (I declined it,
 * missed it, or another device of mine took it): nothing, its ring just stops.
 */
export function endSound(e: EndedCall): EndSound {
  if (e.call.answered_at && e.outcome !== 'answered_elsewhere') return 'end';
  if (e.call.direction !== 'out') return null;
  if (e.local) return 'end';
  return e.outcome === 'answered_elsewhere' ? null : 'busy';
}

/** How long after it was answered a call that talks still chimes: a page opened on a talk long under way stays quiet. */
export const CONNECTED_FRESH_SECS = 20;

/** The call talks for the first time, just now: it chimes. */
export function connectedSound(call: CallView, now: number): boolean {
  return call.phase === 'active' && (call.answered_at == null || now - call.answered_at <= CONNECTED_FRESH_SECS);
}

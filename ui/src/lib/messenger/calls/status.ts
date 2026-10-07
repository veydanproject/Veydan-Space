// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The line under the peer's name on a call's screen, from the store.

import type { CallView } from '../api';
import type { EndedCall } from './callStore.svelte';
import { clock, elapsed, endedKey, phaseKey } from './words';

type Translate = (key: string, params?: Record<string, string>) => string;

/**
 * The phase, or the clock while the call talks; for a call that just
 * ended, how it ended (with how long it was when it was a talk).
 */
export function statusText(call: CallView | null, ended: EndedCall | null, now: number, tr: Translate): string {
  if (call) {
    const key = phaseKey(call);
    return key ? tr(key) : clock(elapsed(call, now));
  }
  if (ended) {
    const words = tr(endedKey(ended.outcome, ended.call.direction));
    return ended.outcome === 'ended' && ended.duration != null ? `${words} · ${clock(ended.duration)}` : words;
  }
  return '';
}

/**
 * What a screen reader is told as the call goes: the phase and how the call
 * ended, as `statusText` says them, but never the clock. A live region is
 * read out at each change, and the clock changes every second.
 */
export function liveText(call: CallView | null, ended: EndedCall | null, tr: Translate): string {
  if (call) {
    const key = phaseKey(call);
    return key ? tr(key) : '';
  }
  return statusText(null, ended, 0, tr);
}

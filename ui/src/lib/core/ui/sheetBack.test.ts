// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { afterBacks, holdBack, resetBacks, type SheetBackEnv } from './sheetBack';
import { BACK_WAIT_MS } from './back';

// The history of a phone: the page, then the places held over it.
function history() {
  let places: string[] = ['page'];
  const listeners = new Set<() => void>();
  const calls: string[] = [];
  const env = (name: string): SheetBackEnv => ({
    push: () => { places = [...places, name]; calls.push(`push ${name}`); },
    stepped: () => places.includes(name),
    back: () => calls.push(`back ${places[places.length - 1]}`),
    events: {
      addEventListener: (_t, f) => listeners.add(f as () => void),
      removeEventListener: (_t, f) => listeners.delete(f as () => void),
    },
  });
  // A back leaves the place, then the page hears of it.
  const pop = () => { places = places.slice(0, -1); for (const f of [...listeners]) f(); };
  return { env, calls, pop, places: () => places };
}

const settle = () => new Promise((r) => setTimeout(r, 0));

describe('a drawer left for another page', () => {
  beforeEach(resetBacks);

  it('closes, its back arrives, and only then the page goes on', async () => {
    const { env, calls, pop, places } = history();
    let closed = 0;
    const release = holdBack(() => closed++, env('drawer'));
    // A row is tapped: the drawer closes, the page waits for the back.
    release();
    void afterBacks().then(() => calls.push('goto'));
    await settle();
    expect(calls).toEqual(['push drawer', 'back drawer']);
    pop();
    await settle();
    expect(calls).toEqual(['push drawer', 'back drawer', 'goto']);
    expect(places()).toEqual(['page']);
    expect(closed).toBe(0);
  });

  it('goes on at once when no back is owed', async () => {
    const { env, calls } = history();
    holdBack(() => {}, env('drawer'));
    void afterBacks().then(() => calls.push('goto'));
    await settle();
    expect(calls).toEqual(['push drawer', 'goto']);
  });

  it('goes on after the system back closed it, without a back of its own', async () => {
    const { env, calls, pop } = history();
    let closed = 0;
    const release = holdBack(() => closed++, env('drawer'));
    pop();
    release();
    void afterBacks().then(() => calls.push('goto'));
    await settle();
    expect(closed).toBe(1);
    expect(calls).toEqual(['push drawer', 'goto']);
  });

  it('goes on when the back never arrives', async () => {
    vi.useFakeTimers();
    try {
      const { env, calls } = history();
      holdBack(() => {}, env('drawer'))();
      void afterBacks().then(() => calls.push('goto'));
      await vi.advanceTimersByTimeAsync(BACK_WAIT_MS);
      expect(calls).toEqual(['push drawer', 'back drawer', 'goto']);
    } finally {
      vi.useRealTimers();
    }
  });

  it('opened and left ten times in a row leaves nothing in the history', async () => {
    const { env, calls, pop, places } = history();
    for (let i = 0; i < 10; i++) {
      holdBack(() => {}, env('drawer'))();
      const went = afterBacks().then(() => calls.push('goto'));
      pop();
      await went;
    }
    expect(places()).toEqual(['page']);
    expect(calls.filter((c) => c === 'goto')).toHaveLength(10);
    expect(calls.filter((c) => c === 'back drawer')).toHaveLength(10);
  });
});

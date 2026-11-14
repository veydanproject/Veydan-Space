// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { ASK_ROUNDS, askRounds, GROUP_EMPTY_END, relaysUnreached, windowsEmpty, type AskSteps } from './askServer';

/**
 * A chat whose relays hold `older` messages under the device's oldest (at 1000),
 * `perAsk` of them per ask, the `matchAt`-th of them (counting from the newest) a match.
 */
function chat({ older = 0, perAsk = 10, matchAt = Infinity, endEarly = false } = {}) {
  let held = 0;
  const asks: number[] = [];
  const loaded: number[] = [];
  let live = true;
  const steps: AskSteps = {
    floor: async () => (held ? 1000 - held : 1000),
    ask: async (before) => {
      asks.push(before);
      held = Math.min(older, held + perAsk);
      return { end: held >= older && (endEarly || held === older) };
    },
    under: async (floor) => ({ count: Math.max(0, held - (1000 - (floor ?? 1000))), floor: 1000 - held }),
    search: async () => (held >= matchAt ? 1 : 0),
    live: () => live,
    loaded: (n) => loaded.push(n),
    now: () => 5000,
  };
  return { steps, asks, loaded, stop: () => (live = false) };
}

describe('askRounds', () => {
  it('asks before the oldest message held, each round lower, and stops on a new match', async () => {
    const c = chat({ older: 100, perAsk: 10, matchAt: 15 });
    expect(await askRounds(c.steps, 0)).toBe('found');
    expect(c.asks).toEqual([1000, 990]);
    expect(c.loaded).toEqual([10, 20]);
  });

  it('stops after the rounds of one press, and a press again goes on from there', async () => {
    const c = chat({ older: 100, perAsk: 10, matchAt: 45 });
    expect(await askRounds(c.steps, 0)).toBe('rounds');
    expect(c.asks).toHaveLength(ASK_ROUNDS);
    expect(await askRounds(c.steps, 0)).toBe('found');
    expect(c.asks).toEqual([1000, 990, 980, 970, 960]);
  });

  it('stops when the relays have nothing older', async () => {
    const c = chat({ older: 12, perAsk: 10 });
    expect(await askRounds(c.steps, 0)).toBe('end');
    expect(c.asks).toEqual([1000, 990]);
    expect(c.loaded).toEqual([10, 12]);
  });

  it('a match found on the last ask wins over the end', async () => {
    const c = chat({ older: 5, perAsk: 10, matchAt: 3 });
    expect(await askRounds(c.steps, 0)).toBe('found');
  });

  it('an empty chat asks before now', async () => {
    const asks: number[] = [];
    const steps: AskSteps = {
      floor: async () => null,
      ask: async (before) => { asks.push(before); return { end: true }; },
      under: async () => ({ count: 0, floor: null }),
      search: async () => 0,
      live: () => true,
      loaded: () => {},
      now: () => 5000,
    };
    expect(await askRounds(steps, 0)).toBe('end');
    expect(asks).toEqual([5000]);
  });

  it('cancelled, it asks no more and does not search', async () => {
    const c = chat({ older: 100, perAsk: 10 });
    let searched = 0;
    const search = c.steps.search;
    c.steps.search = () => { searched++; return search(); };
    const ask = c.steps.ask;
    c.steps.ask = async (before) => { const r = await ask(before); c.stop(); return r; };
    expect(await askRounds(c.steps, 0)).toBe('stopped');
    expect(c.asks).toHaveLength(1);
    expect(searched).toBe(0);
    expect(c.loaded).toEqual([]);
  });

  it('cancelled while the oldest held is read, it does not ask the relays', async () => {
    const c = chat({ older: 100, perAsk: 10 });
    const floor = c.steps.floor;
    c.steps.floor = async () => { const r = await floor(); c.stop(); return r; };
    expect(await askRounds(c.steps, 0)).toBe('stopped');
    expect(c.asks).toEqual([]);
  });

  it('only more than found before counts as new', async () => {
    const c = chat({ older: 30, perAsk: 10, matchAt: 5 });
    // One match was there already: the same one again is not new.
    expect(await askRounds(c.steps, 1)).toBe('end');
  });
});

describe('windowsEmpty', () => {
  /** Asks in a row of a group, each `got` events (null: failed) with the link up or not: the count after each. */
  const run = (asks: [number | null, boolean][]) => {
    let n = 0;
    return asks.map(([got, linked]) => (n = windowsEmpty(n, got, linked)));
  };

  it('two windows the relays answered empty are the end', () => {
    expect(run([[0, true], [0, true]]).at(-1)).toBe(GROUP_EMPTY_END);
  });

  it('a failed ask is no miss, however many: the button stays, and the past comes once the link is back', () => {
    const counts = run([[null, false], [null, false], [null, true], [5, true]]);
    expect(counts).toEqual([0, 0, 0, 0]);
    expect(Math.max(...counts)).toBeLessThan(GROUP_EMPTY_END);
  });

  it('a failed ask between two empty windows neither ends the past nor forgets the first', () => {
    expect(run([[0, true], [null, false], [null, false]])).toEqual([1, 1, 1]);
    expect(run([[0, true], [null, false], [0, true]]).at(-1)).toBe(GROUP_EMPTY_END);
  });

  it('an empty window that came while the link was lost does not count', () => {
    expect(run([[0, false], [0, false], [0, false]])).toEqual([0, 0, 0]);
  });

  it('anything that came, events or messages in view, begins the count again', () => {
    expect(windowsEmpty(1, 3, true)).toBe(0);
    expect(windowsEmpty(1, 0, true, 2)).toBe(0);
  });
});

describe('relaysUnreached', () => {
  it('knows the runtime word for an ask no relay took', () => {
    expect(relaysUnreached('transport: no relay answered')).toBe(true);
    expect(relaysUnreached('no relay connected')).toBe(true);
    expect(relaysUnreached('err.dm_blocked')).toBe(false);
  });
});

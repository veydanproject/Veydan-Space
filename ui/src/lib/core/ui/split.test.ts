// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { SplitDef } from '$lib/core/module';
import { takeSystemBack, type SystemBackEnv } from './split.svelte';

const DEF = { home: '/messenger/section?id=channels' } as SplitDef;

function fake() {
  const log: string[] = [];
  let onback: (() => void) | null = null;
  let resolve!: () => void;
  const taken = new Promise<void>((r) => (resolve = r));
  const env: SystemBackEnv = {
    take: async (fn) => {
      onback = fn;
      log.push('take');
      resolve();
      return () => {
        onback = null;
        log.push('give');
      };
    },
    back: () => log.push('back'),
    leave: (home) => log.push(`leave ${home}`),
  };
  return { env, log, taken, press: () => onback?.() };
}

describe('the system «Назад» beside a list (Android)', () => {
  it('empties the right part, to the list\'s address, as the hidden arrow did', async () => {
    const f = fake();
    takeSystemBack(DEF, () => ({}), f.env);
    await f.taken;
    f.press();
    expect(f.log).toEqual(['take', 'leave /messenger/section?id=channels']);
  });

  it('closes a sheet holding a place in the history first', async () => {
    const f = fake();
    takeSystemBack(DEF, () => ({ emojiDock: true }), f.env);
    await f.taken;
    f.press();
    expect(f.log).toEqual(['take', 'back']);
  });

  it('is the WebView\'s again once given back, also when given back before it was taken', async () => {
    const f = fake();
    const give = takeSystemBack(DEF, () => ({}), f.env);
    give();
    await f.taken;
    await Promise.resolve();
    f.press();
    expect(f.log).toEqual(['take', 'give']);
  });
});

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What is left to set up after the first start: the lines of the card and
// of the wizard's last step.

import { afterEach, describe, expect, it } from 'vitest';
import type { ModuleDef, SetupItem } from '../module';
import { registry } from '../registry';
import { doneCount, setupItems, setupLines } from './setup';

const item = (id: string, order: number, more: Partial<SetupItem> = {}): SetupItem => ({
  id,
  order,
  title: 'setup_sync',
  icon: 'key',
  href: () => `/${id}`,
  done: async () => false,
  ...more,
});

function use(...defs: ModuleDef[]) {
  (registry as unknown as { defs: ModuleDef[] }).defs = defs;
}
afterEach(() => use());

describe('what is left to set up', () => {
  it('is the core\'s sync with what the modules add, in one order, and never the lock of the app', () => {
    use({
      id: 'messenger',
      title: 'setup_sync',
      icon: 'user',
      routes: ['/messenger'],
      nav: [],
      settings: [],
      setup: [item('account', 10), item('recovery-words', 30)],
    });
    expect(setupItems().map((i) => i.id)).toEqual(['account', 'sync', 'recovery-words']);
  });

  it('lists a line with whether it is done, and leaves out one that does not apply', async () => {
    const lines = await setupLines([
      item('a', 1, { done: async () => true }),
      item('b', 2, { visible: async () => false }),
      item('c', 3),
    ]);
    expect(lines.map((l) => [l.item.id, l.done])).toEqual([['a', true], ['c', false]]);
    expect(doneCount(lines)).toBe(1);
  });

  it('counts a line that cannot be asked as not done', async () => {
    const lines = await setupLines([
      item('a', 1, {
        done: async () => {
          throw new Error('no host');
        },
      }),
    ]);
    expect(lines).toEqual([{ item: expect.objectContaining({ id: 'a' }), done: false }]);
  });
});

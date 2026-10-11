// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What is left to set up after the first start: the core's own (sync) and
// what the modules add (the account, the recovery words), in one order. The
// card «Finish setting up» lists all of it. The lock of the app is not
// among them: whether to set one is the person's own business, and its
// place is the settings.

import { api } from '$lib/core/api';
import type { SetupItem } from '$lib/core/module';
import { isMobile } from '$lib/core/platform';
import { registry } from '$lib/core/registry';

/** The settings of sync: a page on a phone, a card of the settings on a computer. */
export const SYNC_HREF = isMobile ? '/settings/sync' : '/settings#sync';

const coreItems: SetupItem[] = [
  {
    id: 'sync',
    order: 20,
    title: 'setup_sync',
    icon: 'refresh-cw',
    href: () => SYNC_HREF,
    step: 'sync',
    done: async () => (await api.sync.status()).joined,
  },
];

/** A line of the list with what is known of it now. */
export interface SetupLine {
  item: SetupItem;
  done: boolean;
}

/** Everything there is to set up in this product, in its order. */
export function setupItems(): SetupItem[] {
  return [...coreItems, ...registry.setup()].sort((a, b) => a.order - b.order);
}

/** The lines to show, each with whether it is done; one that cannot be asked counts as not done. */
export async function setupLines(items: SetupItem[] = setupItems()): Promise<SetupLine[]> {
  const lines = await Promise.all(
    items.map(async (item) => {
      try {
        if (item.visible && !(await item.visible())) return null;
        return { item, done: await item.done() };
      } catch {
        return { item, done: false };
      }
    }),
  );
  return lines.filter((line): line is SetupLine => line !== null);
}

/** How many of the lines are done. */
export function doneCount(lines: SetupLine[]): number {
  return lines.filter((line) => line.done).length;
}

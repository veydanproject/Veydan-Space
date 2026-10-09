// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The phone's way back from a page. `history.length` does not say whether
// there is a page behind this one: it counts the pages ahead too, and a page
// that a tapped notification opened at start replaced the first one. A back
// that went nowhere left a page with a dead arrow, so the back is checked:
// when the address did not change, the page goes to its fallback instead.

import { goto } from '$app/navigation';

/** How long a back is given to arrive before the page goes to its fallback. */
export const BACK_WAIT_MS = 400;

/** How long the fallback is waited for before another way back may start. */
const GO_WAIT_MS = 3000;

/** What a way back leads to: the page behind, the fallback, or nothing (one already under way). */
export type Left = 'back' | 'fallback' | 'busy';

/** The browser, given to a test in its place. */
export interface BackEnv {
  history: { readonly length: number; back(): void };
  location: { readonly href: string };
  events: Pick<EventTarget, 'addEventListener' | 'removeEventListener'>;
  go(route: string): Promise<unknown>;
}

const browser = (): BackEnv => ({
  history: window.history,
  location: window.location,
  events: window,
  go: (route) => goto(route, { replaceState: true }),
});

let busy = false;

/**
 * Back to the page behind this one, or to `fallback` (taking this page's
 * place) when there is none; a fallback given as a function is read only
 * then. A second tap while one way back is under way does nothing: two
 * backs would leave two pages.
 */
export async function leave(fallback: string | (() => string), env: BackEnv = browser()): Promise<Left> {
  if (busy) return 'busy';
  busy = true;
  try {
    const from = env.location.href;
    if (env.history.length > 1) {
      await new Promise<void>((resolve) => {
        const arrived = () => {
          clearTimeout(timer);
          env.events.removeEventListener('popstate', arrived);
          resolve();
        };
        const timer = setTimeout(arrived, BACK_WAIT_MS);
        env.events.addEventListener('popstate', arrived);
        env.history.back();
      });
      // The same address after a back is the same page again (a notification
      // tapped twice), as useless as no back at all.
      if (env.location.href !== from) return 'back';
    }
    // Bounded: a navigation that never ends must not leave every arrow dead.
    const route = typeof fallback === 'string' ? fallback : fallback();
    await Promise.race([env.go(route).catch(() => {}), new Promise((r) => setTimeout(r, GO_WAIT_MS))]);
    return 'fallback';
  } finally {
    busy = false;
  }
}

/**
 * A page's way out that runs once, also when both its back arrow and the
 * end of what it showed ask for it. Only a page that was really left stays
 * left: after a fallback (or a way back of another page under way) the
 * next ask tries again, so the arrow never goes dead.
 */
export function leaveOnce(fallback: () => string, env?: BackEnv): () => Promise<void> {
  let left = false;
  return async () => {
    if (left) return;
    left = true;
    if ((await leave(fallback, env)) !== 'back') left = false;
  };
}

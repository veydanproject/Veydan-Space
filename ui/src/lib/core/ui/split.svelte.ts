// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A wide window (an unfolded Duo, a Fold, a tablet): the sections in a
// column, a module's list and the item opened from it side by side
// (platform-spec 11.7). The shell marks it with `.shell.wide`; every wide
// style lives under that class, never under a bare media query: the shared
// components (the chat window, the lists) stand in the desktop's windows
// too, which are always that wide. A phone lying on its side is not wide:
// too low for two columns. The width is Material's «medium» (600): the
// inner screens of the foldables lie under 840 (Galaxy Z Fold6: 707 × 823
// CSS px, the owner's, 2026-11-27; Pixel Fold alone reaches 852).

import { goto } from '$app/navigation';
import { page } from '$app/state';
import { registry } from '$lib/core/registry';
import type { SplitDef } from '$lib/core/module';
import { leave } from '$lib/core/ui/back';

export const WIDE_QUERY = '(min-width: 600px) and (min-height: 480px)';

/** `url` is the list's own address (`home`, its parameters too: a section of the chats), nothing open beside it. */
export function atHome(url: URL, def: SplitDef): boolean {
  const home = new URL(def.home, url);
  return home.pathname === url.pathname && [...home.searchParams].every(([k, v]) => url.searchParams.get(k) === v);
}

class Split {
  on = $state(false);

  constructor() {
    if (typeof matchMedia !== 'function') return;
    const query = matchMedia(WIDE_QUERY);
    this.on = query.matches;
    // Folding is a resize: the WebView stays, the shell follows (configChanges in the manifest).
    query.addEventListener('change', (e) => (this.on = e.matches));
  }

  /** The list the path stands beside, while the window is wide. */
  of(url: URL) {
    return this.on ? registry.split(url) : undefined;
  }

  /** An item is open beside its list at `url`. */
  beside(url: URL): boolean {
    const def = this.of(url);
    return !!def && !atHome(url, def);
  }

  /**
   * Open a row of a list. Beside an open item the new one takes its place in
   * the history, so back always leaves an empty right part (the list's
   * address), and the next back is the list's own. Narrow, a plain step.
   */
  open(href: string): Promise<void> {
    return goto(href, { replaceState: this.beside(page.url) });
  }

  /** The click of a link to an item's page («+» of a list): the step `open` takes. */
  follow = (e: MouseEvent) => {
    e.preventDefault();
    void this.open((e.currentTarget as HTMLAnchorElement).getAttribute('href') ?? '');
  };

  /** The row of `href` is the one shown beside the list. */
  shows(href: string): boolean {
    return this.on && page.url.pathname === href;
  }
}

export const split = new Split();

/** The system «Назад» and the history, given to a test in their place. */
export interface SystemBackEnv {
  /** Takes the system «Назад» from the WebView until the returned way gives it back. */
  take(onback: () => void): Promise<() => void>;
  back(): void;
  leave(home: string): unknown;
}

const androidBack: SystemBackEnv = {
  take: async (onback) => {
    const { onBackButtonPress } = await import('@tauri-apps/api/app');
    const listener = await onBackButtonPress(onback);
    return () => void listener.unregister();
  },
  back: () => history.back(),
  leave: (home) => leave(home),
};

/**
 * The system «Назад» of an item open beside its list (Android): what the
 * arrow hidden there did. The WebView's own back left the app from a chat
 * beside its list (ROO-1678: its history had nothing it would go back to),
 * so while an item is open beside a list the shell answers the back itself:
 * a sheet holding a place in the history (`state`, core/ui/sheetBack) is
 * closed by its back, otherwise the right part is emptied (`leave` to the
 * list's address, its fallback when there is no page behind). On the list's
 * own address the back is the WebView's again, as on a phone.
 */
export function takeSystemBack(def: SplitDef, state: () => object, env: SystemBackEnv = androidBack): () => void {
  let gone = false;
  let give = () => {};
  void env
    .take(() => {
      if (Object.keys(state()).length) env.back();
      else void env.leave(def.home);
    })
    .then((g) => {
      if (gone) g();
      else give = g;
    })
    .catch(() => {});
  return () => {
    gone = true;
    give();
  };
}

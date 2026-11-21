// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The post whose comments are open beside the feed: it follows the post
// in the feed (a new count, an edit, a reaction), and a removed post
// closes it.
//
// The post is kept as the very object of the feed, never a proxy of it
// (`$state.raw`): a reaction on its way puts a plain object of the post
// into the feed (feedStore.shown), and a proxy made anew of it on every
// write was never the object of the feed, so the thread was written again
// and again until Svelte gave up and the whole window stopped answering
// (the owner on the desktop, 2026-11-21: a reaction from ⋮ of an open
// post's comments).

import { untrack } from 'svelte';
import type { FeedItemView } from './api';

export class OpenThread {
  post = $state.raw<FeedItemView | null>(null);

  /** Made while a component starts (or in an effect root): `items` is the feed shown, `ongone` closes the panel. */
  constructor(items: () => readonly FeedItemView[], ongone: () => void) {
    $effect(() => {
      const t0 = this.post;
      if (!t0) return;
      const fresh = items().find((x) => x.id === t0.id);
      if (fresh?.deleted) untrack(() => { this.post = null; ongone(); });
      else if (fresh && fresh !== t0) untrack(() => (this.post = fresh));
    });
  }
}

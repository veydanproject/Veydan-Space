// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The link the system handed to the app, while its card is on the screen.
// The messenger is the reader of $lib/core/links: it reads once its runtime
// runs, and until then the link waits with the shell.

import { messengerApi } from '../api';
import { messengerStore } from '../store.svelte';
import { shownOf, type Shown } from './incoming';

class IncomingLinks {
  shown = $state<Shown | null>(null);
  /** The address the card is shown at. */
  at = '';
  /** Something in the card was pressed: going elsewhere now is its doing, and closes it. */
  pressed = false;

  read = async (): Promise<boolean> => {
    if (!messengerStore.status?.runtime) return false;
    const view = await messengerApi.links.incoming();
    if (view) {
      this.shown = shownOf(view);
      this.pressed = false;
      this.at = typeof location === 'undefined' ? '' : location.pathname;
    }
    return true;
  };

  close = () => {
    this.shown = null;
  };
}

export const incomingLinks = new IncomingLinks();

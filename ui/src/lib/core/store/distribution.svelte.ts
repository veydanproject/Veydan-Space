// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Where the app came from. One build goes to GitHub and to Google Play; the
// shell tells the store that installed it (crates/shell/src/distribution.rs).
// An app from a store does not check for updates itself and shows the Cloud
// plans without prices and buttons: the store's rules, not ours.

import { api as shared } from '$lib/core/api';

class Distribution {
  /** The store that installed the app; null for any other source and every computer. */
  store = $state<'play' | null>(null);
  readonly fromStore = $derived(this.store !== null);
  private loaded = false;

  async load() {
    if (this.loaded) return;
    this.loaded = true;
    let store: 'play' | null = null;
    try {
      store = (await shared.system.hostInfo()).store ?? null;
    } catch {
      // A host without the command is not a store's.
    }
    this.store = store;
  }
}

export const distribution = new Distribution();

/** Asks the shell once; call it before anything that depends on the store. */
export function loadDistribution(): Promise<void> {
  return distribution.load();
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api } from '$lib/browser/api';
import type { Profile } from '$lib/browser/types';

class ProfilesStore {
  list = $state<Profile[]>([]);
  loading = $state(false);
  loaded = $state(false);
  private _promise: Promise<void> | null = null;
  /** Bumped per refresh so an older response cannot overwrite a newer list. */
  private _generation = 0;

  async ensureLoaded() {
    if (this.loaded) return;
    if (this._promise) return this._promise;
    this._promise = this.refresh().finally(() => { this._promise = null; });
    return this._promise;
  }

  async refresh() {
    const generation = ++this._generation;
    this.loading = true;
    try {
      const list = await api.profiles.list();
      if (generation !== this._generation) return;
      this.list = list;
      this.loaded = true;
    } finally {
      if (generation === this._generation) this.loading = false;
    }
  }

  byWorkspace(id: string): Profile[] {
    return this.list.filter((p) => p.workspace_id === id);
  }
}

export const profilesStore = new ProfilesStore();

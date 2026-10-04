// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api } from '$lib/pass/api';
import type { TotpEntry } from '$lib/pass/types';

class TotpStore {
  list = $state<TotpEntry[]>([]);
  loading = $state(false);
  loaded = $state(false);
  /** Search to apply when the TOTP drawer opens. */
  pendingSearch = $state<string | null>(null);
  private _promise: Promise<void> | null = null;

  async ensureLoaded() {
    if (this.loaded) return;
    if (this._promise) return this._promise;
    this._promise = this.refresh().finally(() => { this._promise = null; });
    return this._promise;
  }

  async refresh() {
    this.loading = true;
    try {
      this.list = await api.totp.list();
      this.loaded = true;
    } finally {
      this.loading = false;
    }
  }

  /** Entries tagged with profile:{id} */
  byProfile(profileId: string): TotpEntry[] {
    const tag = `profile:${profileId}`;
    return this.list.filter((e) => e.tags.includes(tag));
  }

  /** Count for a specific profile — used by Kanban badge */
  countForProfile(profileId: string): number {
    return this.byProfile(profileId).length;
  }

  /** Entries for a workspace: workspace:{id} tag OR profile:{pid} where pid is in profileIds */
  byWorkspace(workspaceId: string, profileIds: string[]): TotpEntry[] {
    const wsTag = `workspace:${workspaceId}`;
    const profileTags = new Set(profileIds.map((id) => `profile:${id}`));
    return this.list.filter(
      (e) => e.tags.includes(wsTag) || e.tags.some((t) => profileTags.has(t))
    );
  }
}

export const totpStore = new TotpStore();

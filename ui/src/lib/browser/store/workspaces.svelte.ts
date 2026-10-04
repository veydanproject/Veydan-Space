// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api } from '$lib/browser/api';
import type { Workspace, CreateWorkspaceRequest, UpdateWorkspaceRequest } from '$lib/browser/types';

class WorkspacesStore {
  list = $state<Workspace[]>([]);
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
      const list = await api.workspaces.list();
      if (generation !== this._generation) return;
      this.list = list;
      this.loaded = true;
    } finally {
      if (generation === this._generation) this.loading = false;
    }
  }

  /** Create a workspace and append it to the cached list. */
  async create(req: CreateWorkspaceRequest): Promise<Workspace> {
    const w = await api.workspaces.create(req);
    this.list = [...this.list, w];
    return w;
  }

  /** Update a workspace and patch it in place. */
  async update(id: string, req: UpdateWorkspaceRequest): Promise<Workspace> {
    const updated = await api.workspaces.update(id, req);
    this.list = this.list.map((w) => (w.id === updated.id ? updated : w));
    return updated;
  }

  /** Delete a workspace and drop it from the cached list. */
  async remove(id: string, mode: 'move_to_default' | 'delete_all'): Promise<void> {
    await api.workspaces.delete(id, mode);
    this.list = this.list.filter((w) => w.id !== id);
  }
}

export const workspacesStore = new WorkspacesStore();

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * Where the product has no browser module — on the phone, and in Notes — the
 * names of workspaces and profiles come from the labels the notes keep
 * (`note_nav`, platform-spec 9.1, 10.2, 10.3). The notes module answers for
 * those two kinds in the catalog of entities there as a fallback — names,
 * colours and nesting only, no actions; where the browser module is present
 * it owns the kinds and this answers nothing.
 */

import { api } from '$lib/notes/api';
import type { EntityKindDef, EntitySummary } from '$lib/core/module';
import type { NavChild } from '$lib/notes/types';

class NavLabels {
  workspaces = $state<NavChild[]>([]);
  profiles = $state<NavChild[]>([]);
  private promise: Promise<void> | null = null;

  ensureLoaded(): Promise<void> {
    this.promise ??= this.refresh().catch(() => { this.promise = null; });
    return this.promise;
  }

  async refresh(): Promise<void> {
    const nav = await api.notes.nav();
    this.workspaces = nav.all_workspaces;
    this.profiles = nav.all_profiles;
  }
}

export const navLabels = new NavLabels();

const summary = (c: NavChild, color: string, parent?: string): EntitySummary => ({
  id: c.id, name: c.name, subtitle: '', status: null, color, parent,
});

export const labelKinds: EntityKindDef[] = [
  {
    kind: 'workspace',
    icon: 'layers',
    label: 'ctx_kind_workspace',
    color: 'var(--success)',
    fallback: true,
    ensureLoaded: () => navLabels.ensureLoaded(),
    list: () => navLabels.workspaces.map((w) => summary(w, w.color)),
    actions: [],
  },
  {
    kind: 'profile',
    icon: 'globe',
    label: 'ctx_kind_profile',
    color: 'var(--accent)',
    fallback: true,
    ensureLoaded: () => navLabels.ensureLoaded(),
    list: () => navLabels.profiles.map((p) => summary(p, 'var(--accent)', p.parent_id ? `workspace:${p.parent_id}` : undefined)),
    actions: [],
  },
];

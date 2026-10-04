// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * Which modules this device keeps on (platform-spec 12), as the shell keeps
 * them in `modules_enabled`: read once with `modules_list`, then followed by
 * `modules://changed`, which every change sends — the Modules section, the
 * first start's question, the messenger's own command. The registry reads it
 * through `registry.enabled`. Outside the app (a plain browser) everything
 * is on.
 */

import { api, MODULES_CHANGED_EVENT, type ModulesView } from '$lib/core/api';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

class ModulesStore {
  /** The modules with a switch of their own, in the order of the product; empty until known. */
  switchable = $state<string[]>([]);
  /** The modules switched off on this device. */
  off = $state<string[]>([]);
  /** A new data file: the first start asks which modules to use. */
  firstRun = $state(false);
  private loading: Promise<void> | null = null;
  private followers: (() => void)[] = [];

  /** Reads the switches once and follows them; resolves when they are known. */
  load(): Promise<void> {
    this.loading ??= this.fetch();
    return this.loading;
  }

  private async fetch(): Promise<void> {
    if (!isTauri) return;
    try {
      this.apply(await api.modules.list());
      const { listen } = await import('@tauri-apps/api/event');
      await listen<ModulesView>(MODULES_CHANGED_EVENT, (e) => this.apply(e.payload));
    } catch (e) {
      // Everything stays on: a shell that cannot answer hides nothing.
      console.error('modules: the switches were not read', e);
    }
  }

  apply(view: ModulesView) {
    this.switchable = view.modules.map((m) => m.id);
    this.off = view.modules.filter((m) => !m.enabled).map((m) => m.id);
    this.firstRun = view.first_run;
    for (const follow of this.followers) follow();
  }

  /** Whether the module is on; a module without a switch of its own always is. */
  isOn(id: string): boolean {
    return !this.off.includes(id);
  }

  /** `fn` runs after every change of the switches. */
  follow(fn: () => void) {
    this.followers.push(fn);
  }

  async set(id: string, enabled: boolean) {
    this.apply(await api.modules.setEnabled(id, enabled));
  }

  /** The answer to the first start's question. */
  async choose(enabled: string[]) {
    this.apply(await api.modules.choose(enabled));
  }
}

export const modulesStore = new ModulesStore();

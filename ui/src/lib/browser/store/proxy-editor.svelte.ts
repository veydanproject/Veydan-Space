// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { proxiesStore } from '$lib/browser/store/proxies.svelte';

/** The proxy editor opened from outside the proxies page (a connection's proxy chip in the terminal). */
class ProxyEditorStore {
  /** Id of the proxy being edited; null when the editor is closed. */
  editing = $state<string | null>(null);

  open(id: string) {
    this.editing = id;
  }

  close() {
    this.editing = null;
  }
}

export const proxyEditor = new ProxyEditorStore();

/** The proxy the editor shows, from the store's list. */
export function editingProxy() {
  return proxyEditor.editing ? proxiesStore.list.find((p) => p.id === proxyEditor.editing) ?? null : null;
}

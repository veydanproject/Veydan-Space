// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The list of Tor instances as the UI follows it: read once, then replaced by
// every `tor://instances` event.

import { api } from '$lib/tor/api';
import type { InstanceInfo } from '$lib/tor/types';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** Calls `onChange` with the current list and with each new one; returns the way to stop. */
export function watchInstances(onChange: (list: InstanceInfo[]) => void): () => void {
  let stopped = false;
  let unlisten: (() => void) | null = null;
  // An event that comes before the first read is newer than that read.
  let fresh = false;

  api.tor.instances().then((list) => {
    if (!stopped && !fresh) onChange(list);
  }).catch(() => {});

  if (isTauri) {
    import('@tauri-apps/api/event').then(async ({ listen }) => {
      const fn = await listen<InstanceInfo[]>('tor://instances', (e) => {
        fresh = true;
        onChange(e.payload);
      });
      if (stopped) fn();
      else unlisten = fn;
    }).catch(() => {});
  }

  return () => {
    stopped = true;
    unlisten?.();
    unlisten = null;
  };
}

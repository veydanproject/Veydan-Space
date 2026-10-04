// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Mobile view of the shell's API: the sync commands and events as the phone
// screens use them. The notes' view is $lib/notes/mobile/api.

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { api as shared } from '$lib/core/api';
import { formatError } from '$lib/core/utils';
import type { Key } from '$lib/core/mobile/i18n';
import type { SyncConfig, SyncProgress, SyncStatus } from '$lib/core/api';

export { formatError };
export type { SyncConfig, SyncProgress, SyncStatus };

export type SyncProbe = 'empty' | 'vault' | 'foreign';

// ── Events ───────────────────────────────────────────────────────────────────

// Outside Tauri (the phone UI's dev server in a plain browser) there are no
// events: a listener is a no-op instead of an uncaught TypeError.
const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
const noop: UnlistenFn = () => {};

/** Runs `cb` when sync applied remote changes to one of `entities` ("note", "totp", ...). */
export function onSyncChanged(entities: string[], cb: () => void): Promise<UnlistenFn> {
  if (!isTauri) return Promise.resolve(noop);
  return listen<string[]>('sync://data-changed', (e) => {
    if (e.payload.some((x) => entities.includes(x))) cb();
  });
}

export function onSyncStatus(cb: () => void): Promise<UnlistenFn> {
  if (!isTauri) return Promise.resolve(noop);
  return listen('sync://status', cb);
}

export function onSyncProgress(cb: (p: SyncProgress) => void): Promise<UnlistenFn> {
  if (!isTauri) return Promise.resolve(noop);
  return listen<SyncProgress>('sync://progress', (e) => cb(e.payload));
}

export function syncProgressText(
  t: (key: Key, vars?: Record<string, string>) => string,
  p: SyncProgress,
): string {
  // The phase key is built from the backend's phase name, which the key union
  // cannot express — asserted the same way as other dynamic keys in the app.
  const phase = t(`settings_sync_phase_${p.phase}` as Key);
  if (p.total > 0) {
    return t('settings_sync_progress_files', {
      phase,
      current: String(p.current),
      total: String(p.total),
      percent: String(p.percent),
    });
  }
  return t('settings_sync_progress', { phase, percent: String(p.percent) });
}

// ── API ──────────────────────────────────────────────────────────────────────

// ── API ──────────────────────────────────────────────────────────────────────

export const api = {
  sync: {
    getConfig: () => shared.sync.getConfig(),
    setConfig: (cfg: SyncConfig) => shared.sync.setConfig(cfg),
    probe: () => shared.sync.probe(),
    createVault: (passphrase: string) => shared.sync.createVault(passphrase),
    joinVault: (passphrase: string) => shared.sync.joinVault(passphrase),
    leave: () => shared.sync.leave(),
    status: () => shared.sync.status(),
    runNow: () => shared.sync.runNow(),
    trigger: () => shared.sync.trigger(),
  },
};

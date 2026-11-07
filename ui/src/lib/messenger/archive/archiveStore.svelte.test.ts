// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ARCHIVE_GATE_KEY, holdArchiveGate } from '$lib/core/ui/archiveGate';
import { GATE_BUDGET_MS } from './archive';
import type { ArchiveStatus } from './types';

const status = vi.fn<() => Promise<ArchiveStatus>>();
vi.mock('../api', () => ({ messengerApi: { archive: { status: () => status() } } }));

const restoring: ArchiveStatus = {
  available: true,
  progress: { phase: 'restoring', done: 1, total: 4, bytes_done: 100, bytes_total: 1000, month: '2026-10' },
  size_bytes: 1000,
  from_month: '2026-01',
  to_month: '2026-10',
  paused: null,
  restoring_scopes: ['dm', 'group:g1'],
  too_large: [],
  not_backed_up: [],
};

async function freshStore() {
  vi.resetModules();
  const mod = await import('./archiveStore.svelte');
  return mod;
}

/** The tests run without a browser: the session storage and the window the store and the gate use, made up. */
function browserGlobals() {
  const kept = new Map<string, string>();
  vi.stubGlobal('sessionStorage', {
    getItem: (k: string) => kept.get(k) ?? null,
    setItem: (k: string, v: string) => void kept.set(k, v),
    removeItem: (k: string) => void kept.delete(k),
    clear: () => kept.clear(),
  });
  vi.stubGlobal('window', new EventTarget());
}

describe('archiveStore', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    browserGlobals();
    status.mockReset();
    status.mockResolvedValue(restoring);
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('arms the gate written by the cloud pages and lets go after the budget', async () => {
    holdArchiveGate();
    const { archiveStore } = await freshStore();
    expect(archiveStore.armed).toBe(true);
    await archiveStore.load();
    expect(archiveStore.blocking).toBe(true);
    vi.advanceTimersByTime(GATE_BUDGET_MS + 1);
    expect(archiveStore.armed).toBe(false);
    expect(sessionStorage.getItem(ARCHIVE_GATE_KEY)).toBeNull();
    // The restore goes on behind: the banner's state, not the gate's.
    expect(archiveStore.restoring).toBe(true);
    expect(archiveStore.blocking).toBe(false);
  });

  it('does not arm on a stale key', async () => {
    sessionStorage.setItem(ARCHIVE_GATE_KEY, String(Date.now() - GATE_BUDGET_MS - 1));
    const { archiveStore } = await freshStore();
    expect(archiveStore.armed).toBe(false);
  });

  it('lets go on archive.synced and asks the status again', async () => {
    holdArchiveGate();
    const { archiveStore } = await freshStore();
    await archiveStore.load();
    expect(status).toHaveBeenCalledTimes(1);
    archiveStore.handleEvent({ name: 'archive.synced', payload: {} });
    expect(archiveStore.armed).toBe(false);
    expect(archiveStore.progress.phase).toBe('idle');
    expect(archiveStore.isRestoring('dm')).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(status).toHaveBeenCalledTimes(2);
  });

  it('lets go when the device has no storage, and progress does not make one up', async () => {
    holdArchiveGate();
    status.mockResolvedValue({ ...restoring, available: false, paused: { why: 'no_storage' } });
    const { archiveStore } = await freshStore();
    await archiveStore.load();
    expect(archiveStore.armed).toBe(false);
    archiveStore.handleEvent({ name: 'archive.progress', payload: restoring.progress });
    expect(archiveStore.available).toBe(false);
    expect(archiveStore.restoring).toBe(false);
  });

  it('fills the status from progress and tells which chats still wait', async () => {
    const { archiveStore } = await freshStore();
    archiveStore.handleEvent({ name: 'archive.progress', payload: restoring.progress });
    expect(archiveStore.status?.available).toBe(true);
    expect(archiveStore.progress.month).toBe('2026-10');
    expect(archiveStore.percent).toBe(10);
    // The scopes come with the status, asked once the phase changed.
    expect(archiveStore.isRestoring('dm')).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(status).toHaveBeenCalledTimes(1);
    expect(archiveStore.isRestoring('dm')).toBe(true);
    expect(archiveStore.isRestoring('group:g2')).toBe(false);
    // A new month under way asks again, but not before the throttle.
    archiveStore.handleEvent({ name: 'archive.progress', payload: { ...restoring.progress, month: '2026-09', bytes_done: 200 } });
    archiveStore.handleEvent({ name: 'archive.progress', payload: { ...restoring.progress, month: '2026-09', bytes_done: 300 } });
    expect(status).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(5_001);
    expect(status).toHaveBeenCalledTimes(2);
  });

  it('reset clears the hold and the status', async () => {
    holdArchiveGate();
    const { archiveStore } = await freshStore();
    await archiveStore.load();
    expect(archiveStore.armed).toBe(true);
    archiveStore.reset();
    expect(archiveStore.armed).toBe(false);
    expect(archiveStore.status).toBeNull();
    expect(sessionStorage.getItem(ARCHIVE_GATE_KEY)).toBeNull();
  });
});

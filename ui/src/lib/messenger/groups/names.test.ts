// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A stranger's name: a profile this device has never seen is asked of the
// relays once, is not «not found» while it is awaited, and takes its name
// when `profile.updated` brings it; a relay back after none asks again.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const FRESH = 'cc'.repeat(32);

const api = vi.hoisted(() => {
  const state = { cache: {} as Record<string, { name: string } | null> };
  return {
    state,
    get: vi.fn(async (pk: string) => state.cache[pk] ?? null),
    request: vi.fn(async () => {}),
  };
});
vi.mock('../api', () => ({
  messengerApi: { profiles: { get: api.get, request: api.request } },
  profileLabel: (p: { name?: string } | null) => p?.name ?? '',
  contactLabel: (c: { pubkey: string }) => c.pubkey,
}));
vi.mock('../store.svelte', () => ({ messengerStore: { identity: null, ownProfile: null, contacts: [] } }));

const { nameStore, shortKey } = await import('./names.svelte');

const asks = (pk: string) => api.request.mock.calls.filter((c) => (c as unknown[])[0] === pk).length;
const settle = async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); };

describe('a stranger never seen', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    api.state.cache = {};
    api.get.mockClear();
    api.request.mockClear();
  });
  afterEach(() => vi.useRealTimers());

  it('is asked of the relays once, is not missing while awaited, and takes the name that comes', async () => {
    expect(nameStore.label(FRESH)).toBe(shortKey(FRESH));
    await settle();
    expect(api.request).toHaveBeenCalledTimes(1);
    expect(api.request).toHaveBeenCalledWith(FRESH);
    expect(nameStore.missing(FRESH)).toBe(false);

    api.state.cache[FRESH] = { name: 'Борис' };
    nameStore.refresh(FRESH);
    await settle();
    expect(nameStore.label(FRESH)).toBe('Борис');
    expect(nameStore.missing(FRESH)).toBe(false);
    expect(api.request).toHaveBeenCalledTimes(1);
  });

  it('counts as not found only once the wait is over', async () => {
    const other = 'dd'.repeat(32);
    nameStore.label(other);
    await settle();
    expect(nameStore.missing(other)).toBe(false);
    vi.advanceTimersByTime(15_000);
    expect(nameStore.missing(other)).toBe(true);
  });

  it('is asked again when a relay is back, after the ask failed offline', async () => {
    const offline = 'ee'.repeat(32);
    api.request.mockRejectedValueOnce('no relay connected');
    nameStore.label(offline);
    await settle();
    expect(asks(offline)).toBe(1);
    expect(nameStore.missing(offline)).toBe(true);

    nameStore.reconnected();
    await settle();
    expect(asks(offline)).toBe(2);
    expect(nameStore.missing(offline)).toBe(false);

    api.state.cache[offline] = { name: 'Вера' };
    nameStore.refresh(offline);
    await settle();
    expect(nameStore.label(offline)).toBe('Вера');
  });

  it('is asked again when a relay is back, after the wait ran out', async () => {
    const lost = 'ff'.repeat(32);
    nameStore.label(lost);
    await settle();
    vi.advanceTimersByTime(10_000);
    nameStore.reconnected();
    await settle();
    expect(asks(lost)).toBe(2);
    // The first ask's timer does not end the second's wait.
    vi.advanceTimersByTime(5_000);
    expect(nameStore.missing(lost)).toBe(false);
    vi.advanceTimersByTime(10_000);
    expect(nameStore.missing(lost)).toBe(true);
  });
});

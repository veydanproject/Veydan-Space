// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Who is shown online follows the runtime's list: a contact removed or
// blocked goes at once, a late beat does not bring them back, and with the
// switch off nobody is shown.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const ALICE = 'aa'.repeat(32);
const BOB = 'bb'.repeat(32);

const api = vi.hoisted(() => {
  const state = { list: [] as { peer: string; seen_at: number; online_until: number }[] };
  return { state, list: vi.fn(async () => state.list.map((p) => ({ ...p }))) };
});
vi.mock('../api', () => ({
  messengerApi: { presence: { list: api.list, foreground: vi.fn(async () => {}) } },
}));

const { presenceStore } = await import('./presenceStore.svelte');

const beat = (peer: string, seen_at: number) => ({
  name: 'presence.updated',
  payload: { peer, seen_at, online_until: seen_at + 80 },
});
const settle = () => new Promise((r) => setTimeout(r, 0));

describe('the presence list', () => {
  beforeEach(async () => {
    presenceStore.reset();
    api.list.mockClear();
    api.state.list = [
      { peer: ALICE, seen_at: 100, online_until: 180 },
      { peer: BOB, seen_at: 100, online_until: 180 },
    ];
    await presenceStore.load();
  });

  it('takes a beat of a contact it shows', () => {
    presenceStore.handleEvent(beat(ALICE, 130));
    expect(presenceStore.map[ALICE]).toEqual({ seen_at: 130, online_until: 210 });
    expect(api.list).toHaveBeenCalledTimes(1);
  });

  it('drops a contact removed or blocked, and a late beat asks the list again', async () => {
    api.state.list = [{ peer: ALICE, seen_at: 100, online_until: 180 }];
    presenceStore.handleEvent({ name: 'dm.relationship', payload: { peer: BOB } });
    await settle();
    expect(presenceStore.map[BOB]).toBeUndefined();
    presenceStore.handleEvent(beat(BOB, 140));
    await settle();
    expect(presenceStore.map[BOB]).toBeUndefined();
    expect(api.list).toHaveBeenCalledTimes(3);
  });

  it('shows a contact the list has, once its first beat comes', async () => {
    const carol = 'cc'.repeat(32);
    api.state.list.push({ peer: carol, seen_at: 150, online_until: 230 });
    presenceStore.handleEvent(beat(carol, 150));
    await settle();
    expect(presenceStore.map[carol]).toEqual({ seen_at: 150, online_until: 230 });
  });

  it('shows nobody with the switch off, whatever comes late', async () => {
    presenceStore.setSharing(false);
    expect(presenceStore.map).toEqual({});
    presenceStore.handleEvent(beat(ALICE, 130));
    presenceStore.handleEvent({ name: 'presence.keys_changed', payload: { peer: ALICE } });
    await settle();
    expect(presenceStore.map).toEqual({});
    expect(api.list).toHaveBeenCalledTimes(1);
    presenceStore.setSharing(true);
    await settle();
    expect(Object.keys(presenceStore.map).sort()).toEqual([ALICE, BOB]);
  });
});

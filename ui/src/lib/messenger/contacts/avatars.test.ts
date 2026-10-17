// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A face the runtime did not fetch (a stranger then, a fetch that failed)
// gets its picture without a restart: asked again when the reason may have
// changed, and now and then while a face shows it.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => {
  const state = { answers: {} as Record<string, string | null | Error> };
  return {
    state,
    cached: vi.fn(async (url: string) => {
      const a = state.answers[url];
      if (a instanceof Error) throw a;
      return a ?? null;
    }),
  };
});
vi.mock('../api', () => ({ messengerApi: { avatar: { cached: api.cached } } }));

const { avatarStore, AVATAR_RETRY_MS, AVATAR_SETTLE_MS } = await import('./avatars.svelte');

const URL_A = 'https://media.example/' + 'a'.repeat(64);
const URL_B = 'https://media.example/' + 'b'.repeat(64);
const PIC = 'data:image/jpeg;base64,AAAA';
const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};
const asked = (url: string) => api.cached.mock.calls.filter(([u]) => u === url).length;

/** Read as a template does, and let the first ask answer. */
async function read(url: string) {
  avatarStore.get(url);
  await settle();
}

describe('pictures still on their way', () => {
  let unshow: (() => void)[] = [];

  beforeEach(() => {
    vi.useFakeTimers();
    avatarStore.reset();
    api.cached.mockClear();
    api.state.answers = {};
  });

  afterEach(() => {
    unshow.forEach((u) => u());
    unshow = [];
    avatarStore.reset();
    vi.useRealTimers();
  });

  it('asks again once a stranger becomes a contact', async () => {
    unshow.push(avatarStore.show(URL_A));
    await read(URL_A);
    expect(avatarStore.state(URL_A)).toBe('pending');
    expect(asked(URL_A)).toBe(1);

    // Accepted: the runtime now fetches it, and has it in its cache.
    api.state.answers[URL_A] = PIC;
    avatarStore.handleEvent({ name: 'dm.relationship', payload: { peer: 'aa'.repeat(32) } });
    avatarStore.handleEvent({ name: 'contacts.updated', payload: {} });
    expect(asked(URL_A)).toBe(1);
    await vi.advanceTimersByTimeAsync(AVATAR_SETTLE_MS);
    expect(asked(URL_A)).toBe(2);
    expect(avatarStore.get(URL_A)).toBe(PIC);
  });

  it('leaves a waiting address not on the page to its next read', async () => {
    await read(URL_B);
    expect(avatarStore.state(URL_B)).toBe('pending');
    avatarStore.handleEvent({ name: 'group.updated', payload: {} });
    await vi.advanceTimersByTimeAsync(AVATAR_SETTLE_MS);
    expect(asked(URL_B)).toBe(1);
    await read(URL_B);
    expect(asked(URL_B)).toBe(2);
  });

  it('asks a face on the page again now and then, and stops once nothing waits', async () => {
    unshow.push(avatarStore.show(URL_A));
    await read(URL_A);
    await vi.advanceTimersByTimeAsync(AVATAR_RETRY_MS);
    expect(asked(URL_A)).toBe(2);
    expect(avatarStore.state(URL_A)).toBe('pending');

    // The fetch that failed went through on the runtime's next try.
    api.state.answers[URL_A] = PIC;
    await vi.advanceTimersByTimeAsync(AVATAR_RETRY_MS);
    expect(asked(URL_A)).toBe(3);
    expect(avatarStore.get(URL_A)).toBe(PIC);
    await vi.advanceTimersByTimeAsync(3 * AVATAR_RETRY_MS);
    expect(asked(URL_A)).toBe(3);
  });

  it('asks a refused address again, and never takes a picture away', async () => {
    unshow.push(avatarStore.show(URL_A), avatarStore.show(URL_B));
    api.state.answers[URL_A] = new Error('vault_locked');
    api.state.answers[URL_B] = PIC;
    await read(URL_A);
    await read(URL_B);
    expect(avatarStore.state(URL_A)).toBeNull();

    api.state.answers[URL_A] = PIC;
    api.state.answers[URL_B] = new Error('vault_locked');
    avatarStore.handleEvent({ name: 'profile.updated', payload: { pubkey: 'bb'.repeat(32) } });
    await vi.advanceTimersByTimeAsync(AVATAR_SETTLE_MS);
    expect(avatarStore.get(URL_A)).toBe(PIC);
    expect(asked(URL_B)).toBe(1);

    avatarStore.handleEvent({ name: 'avatar.ready', payload: { url: URL_B } });
    await settle();
    expect(avatarStore.get(URL_B)).toBe(PIC);
  });

  it('is not woken by events that change nothing of who is known', async () => {
    unshow.push(avatarStore.show(URL_A));
    await read(URL_A);
    avatarStore.handleEvent({ name: 'dm.message', payload: {} });
    avatarStore.handleEvent({ name: 'transfer.progress', payload: {} });
    await vi.advanceTimersByTimeAsync(AVATAR_SETTLE_MS);
    expect(asked(URL_A)).toBe(1);
  });

  it('stops asking for a face once it left the page', async () => {
    const hide = avatarStore.show(URL_A);
    const again = avatarStore.show(URL_A);
    await read(URL_A);
    hide();
    await vi.advanceTimersByTimeAsync(AVATAR_RETRY_MS);
    expect(asked(URL_A), 'still shown by the other face').toBe(2);
    again();
    await vi.advanceTimersByTimeAsync(AVATAR_RETRY_MS);
    expect(asked(URL_A)).toBe(2);
  });
});

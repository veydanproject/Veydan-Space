// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The cover of a channel I am in is read by the module once per address
// and shown as a `data:` URL; a picture that is one already is shown as it
// is; a card of a channel not joined leaves its address to the runtime.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({ cover: vi.fn(async (_id: string): Promise<string | null> => 'data:image/jpeg;base64,AAAA') }));
vi.mock('./api', () => ({ channelsApi: { cover: api.cover } }));

const { coverStore } = await import('./coverStore.svelte');
const { cover } = await import('./cover');

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('the covers of channels', () => {
  beforeEach(() => api.cover.mockClear());

  it('are asked once per channel and address, and shown once read', async () => {
    const url = 'https://h1.example/' + 'a'.repeat(64);
    expect(coverStore.src('c1', url)).toBeNull();
    await flush();
    expect(coverStore.src('c1', url)).toBe('data:image/jpeg;base64,AAAA');
    expect(coverStore.src('c1', url)).toBe('data:image/jpeg;base64,AAAA');
    expect(api.cover).toHaveBeenCalledTimes(1);
  });

  it('are asked again for a new address of the same channel', async () => {
    const url = 'https://h1.example/' + 'b'.repeat(64);
    coverStore.src('c1', url);
    await flush();
    expect(api.cover).toHaveBeenCalledTimes(1);
    expect(coverStore.src('c1', url)).toBe('data:image/jpeg;base64,AAAA');
  });

  it('a failure shows initials and is not asked in a loop', async () => {
    api.cover.mockRejectedValueOnce(new Error('no'));
    const url = 'https://h1.example/' + 'c'.repeat(64);
    expect(coverStore.src('c2', url)).toBeNull();
    await flush();
    expect(coverStore.src('c2', url)).toBeNull();
    expect(coverStore.src('c2', url)).toBeNull();
    expect(api.cover).toHaveBeenCalledTimes(1);
  });

  it('cover(): a data URL as it is, a joined channel through the store, a card by its address', () => {
    expect(cover('data:image/png;base64,xx', 'c1')).toEqual({ url: null, src: 'data:image/png;base64,xx' });
    expect(cover('https://h/x')).toEqual({ url: 'https://h/x', src: null });
    expect(cover('https://h/' + 'a'.repeat(64), 'c1').url).toBeNull();
    expect(cover(null, 'c1')).toEqual({ url: null, src: null });
  });
});

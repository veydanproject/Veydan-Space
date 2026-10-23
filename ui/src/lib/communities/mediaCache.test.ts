// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The files of the channels fetched once while the app runs: by their hash,
// up to a number of bytes, the least recently shown going first.

import { describe, expect, it, vi } from 'vitest';
import { MediaCache, mediaKey } from './mediaCache';

const url = (n: number) => 'x'.repeat(n);

describe('the files fetched', () => {
  it('are kept by the hash of the file, else by the message', () => {
    expect(mediaKey({ community_id: 'c', id: 'm1' }, { sha256: 'ab' })).toBe('sha:ab');
    expect(mediaKey({ community_id: 'c', id: 'm2' }, { sha256: 'ab' })).toBe('sha:ab');
    expect(mediaKey({ community_id: 'c', id: 'm1' }, null)).toBe('id:c:m1');
  });

  it('are fetched once for everyone who asks meanwhile, and not again', async () => {
    const cache = new MediaCache(1000, 500);
    const load = vi.fn(async () => ({ path: '/p', src: url(100) }));
    const [a, b] = await Promise.all([cache.fetch('k', load), cache.fetch('k', load)]);
    expect(a).toBe(b);
    await cache.fetch('k', load);
    expect(load).toHaveBeenCalledTimes(1);
  });

  it('keep no failure', async () => {
    const cache = new MediaCache(1000, 500);
    await expect(cache.fetch('k', async () => { throw new Error('no host'); })).rejects.toThrow();
    const load = vi.fn(async () => ({ path: '/p', src: null }));
    await cache.fetch('k', load);
    expect(load).toHaveBeenCalledTimes(1);
  });

  it('stay within their bytes, the least recently shown going first', () => {
    const cache = new MediaCache(1000, 500);
    cache.put('a', { path: '/a', src: url(400) });
    cache.put('b', { path: '/b', src: url(400) });
    cache.get('a');
    cache.put('c', { path: '/c', src: url(400) });
    expect(cache.bytes).toBeLessThanOrEqual(1000);
    expect(cache.get('b')).toBeUndefined();
    expect(cache.get('a')?.path).toBe('/a');
    expect(cache.get('c')?.path).toBe('/c');
  });

  it('keep only the place of a file too large to hold, and fetch its picture again', async () => {
    const cache = new MediaCache(1000, 500);
    cache.put('big', { path: '/big', src: url(800) });
    expect(cache.bytes).toBe(0);
    expect(cache.get('big')).toMatchObject({ path: '/big', src: null, large: true });
    const load = vi.fn(async () => ({ path: '/big', src: url(800) }));
    expect((await cache.fetch('big', load)).src).toHaveLength(800);
    expect(load).toHaveBeenCalledTimes(1);
  });
});

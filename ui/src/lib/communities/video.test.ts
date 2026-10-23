// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A video of a post before it plays: its length, its box, a frame taken
// once and kept.

import { describe, expect, it, vi } from 'vitest';
import { FrameCache, frameBox, videoLength } from './video';

describe('the length of a video', () => {
  it('reads as a clock', () => {
    expect(videoLength(0)).toBe('0:00');
    expect(videoLength(7_400)).toBe('0:07');
    expect(videoLength(42_910)).toBe('0:43');
    expect(videoLength(725_000)).toBe('12:05');
    expect(videoLength(3_723_000)).toBe('1:02:03');
    expect(videoLength(-5)).toBe('0:00');
  });
});

describe('the box of a video alone', () => {
  it('keeps its proportions within the side', () => {
    expect(frameBox([1920, 1080])).toEqual({ width: 420, ratio: '1920 / 1080' });
    // A phone's upright video: as tall as the side, narrower.
    expect(frameBox([960, 2142])).toEqual({ width: 188, ratio: '960 / 2142' });
    expect(frameBox([500, 500])).toEqual({ width: 420, ratio: '500 / 500' });
  });

  it('is 16:9 when the size is not known', () => {
    expect(frameBox(undefined)).toEqual({ width: 420, ratio: '16 / 9' });
    expect(frameBox([0, 0])).toEqual({ width: 420, ratio: '16 / 9' });
  });
});

describe('the frames taken here', () => {
  it('are taken once for everyone who asks meanwhile, and kept', async () => {
    const cache = new FrameCache();
    const grab = vi.fn(async () => ({ src: 'data:image/jpeg;base64,AA', durationMs: 4200 }));
    const [a, b] = await Promise.all([cache.take('sha:1', grab), cache.take('sha:1', grab)]);
    expect(a).toEqual(b);
    expect(await cache.take('sha:1', grab)).toEqual({ src: 'data:image/jpeg;base64,AA', durationMs: 4200 });
    expect(grab).toHaveBeenCalledTimes(1);
    expect(cache.peek('sha:1')?.durationMs).toBe(4200);
  });

  it('remember a video that gave none, and a failure, so it is not decoded again and again', async () => {
    const cache = new FrameCache();
    const none = vi.fn(async () => null);
    expect(await cache.take('sha:2', none)).toBeNull();
    expect(await cache.take('sha:2', none)).toBeNull();
    expect(none).toHaveBeenCalledTimes(1);
    expect(cache.peek('sha:2')).toBeNull();
    expect(cache.peek('sha:3')).toBeUndefined();
    expect(await cache.take('sha:3', async () => { throw new Error('no decoder'); })).toBeNull();
  });

  it('keep the most recent ones', async () => {
    const cache = new FrameCache(2);
    for (const k of ['a', 'b', 'c']) await cache.take(k, async () => ({ src: k, durationMs: null }));
    expect(cache.peek('a')).toBeUndefined();
    expect(cache.peek('c')?.src).toBe('c');
  });
});

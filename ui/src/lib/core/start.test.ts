// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { afterEach, describe, expect, it, vi } from 'vitest';
import { answerWithin, NoAnswer, retryOnce, started, wrongPlatform } from './start';

describe('wrongPlatform', () => {
  it('accepts the desktop UI on a computer and the phone UI on a phone', () => {
    for (const os of ['linux', 'windows', 'macos']) expect(wrongPlatform(false, os), os).toBe(false);
    for (const os of ['android', 'ios']) expect(wrongPlatform(true, os), os).toBe(false);
  });

  it('refuses the phone UI in a desktop window and the desktop UI on a phone', () => {
    for (const os of ['linux', 'windows', 'macos']) expect(wrongPlatform(true, os), os).toBe(true);
    for (const os of ['android', 'ios']) expect(wrongPlatform(false, os), os).toBe(true);
  });

  it('accuses nothing without an answer of the host', () => {
    expect(wrongPlatform(true, undefined)).toBe(false);
    expect(wrongPlatform(false, '')).toBe(false);
  });
});

function memory() {
  const items = new Map<string, string>();
  return {
    getItem: (k: string) => items.get(k) ?? null,
    setItem: (k: string, v: string) => void items.set(k, v),
    removeItem: (k: string) => void items.delete(k),
  };
}

describe('answerWithin', () => {
  afterEach(() => vi.useRealTimers());

  it('passes an answer on', async () => {
    await expect(answerWithin(Promise.resolve(7), 1000)).resolves.toBe(7);
  });

  it('passes a refusal on', async () => {
    await expect(answerWithin(Promise.reject(new Error('refused')), 1000)).rejects.toThrow('refused');
  });

  it('gives up on a command that is never answered', async () => {
    vi.useFakeTimers();
    const never = new Promise<number>(() => {});
    const result = answerWithin(never, 20_000);
    const seen = expect(result).rejects.toBeInstanceOf(NoAnswer);
    await vi.advanceTimersByTimeAsync(20_000);
    await seen;
  });
});

describe('retryOnce', () => {
  it('reloads once per session, then lets the page say why', () => {
    const flags = memory();
    const reload = vi.fn();
    expect(retryOnce(reload, flags)).toBe(true);
    expect(retryOnce(reload, flags)).toBe(false);
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('may reload again after a start that worked', () => {
    const flags = memory();
    const reload = vi.fn();
    expect(retryOnce(reload, flags)).toBe(true);
    started(flags);
    expect(retryOnce(reload, flags)).toBe(true);
    expect(reload).toHaveBeenCalledTimes(2);
  });

  it('does not reload without storage', () => {
    const reload = vi.fn();
    expect(retryOnce(reload, null)).toBe(false);
    expect(reload).not.toHaveBeenCalled();
  });
});

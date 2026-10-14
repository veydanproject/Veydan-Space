import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { copySecret } from './clipboard';

let content = '';
let failWrite = false;

beforeEach(() => {
  vi.useFakeTimers();
  content = '';
  failWrite = false;
  vi.stubGlobal('navigator', {
    clipboard: {
      writeText: vi.fn(async (v: string) => {
        if (failWrite) throw new Error('denied');
        content = v;
      }),
      readText: vi.fn(async () => content),
    },
  });
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('copySecret', () => {
  it('clears the clipboard after 30 s when it still holds the secret', async () => {
    await copySecret('s3cret');
    expect(content).toBe('s3cret');
    await vi.advanceTimersByTimeAsync(29_999);
    expect(content).toBe('s3cret');
    await vi.advanceTimersByTimeAsync(1);
    expect(content).toBe('');
  });

  it('leaves something copied since alone', async () => {
    await copySecret('s3cret');
    content = 'other';
    await vi.advanceTimersByTimeAsync(30_000);
    expect(content).toBe('other');
  });

  it('honours clearAfterMs, and 0 keeps the value', async () => {
    await copySecret('a', { clearAfterMs: 1000 });
    await vi.advanceTimersByTimeAsync(1000);
    expect(content).toBe('');
    await copySecret('b', { clearAfterMs: 0 });
    await vi.advanceTimersByTimeAsync(60_000);
    expect(content).toBe('b');
  });

  it('uses the fallback when the clipboard refuses', async () => {
    failWrite = true;
    const fallback = vi.fn(async () => {});
    await copySecret('x', { fallback });
    expect(fallback).toHaveBeenCalledWith('x');
  });

  it('survives an unreadable clipboard at clear time', async () => {
    await copySecret('x');
    (navigator.clipboard.readText as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('denied'));
    await vi.advanceTimersByTimeAsync(30_000);
    expect(content).toBe('x');
  });
});

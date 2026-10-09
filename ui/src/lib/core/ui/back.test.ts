// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('$app/navigation', () => ({ goto: vi.fn(async () => {}) }));

import { BACK_WAIT_MS, leave, leaveOnce, type BackEnv } from './back';

/**
 * A browser of pages: `back` moves to the page behind (with a popstate a
 * moment later), or does nothing when this page is the first of the app,
 * whatever `history.length` says.
 */
function browser(pages: string[], at = pages.length - 1, length = pages.length) {
  const events = new EventTarget();
  const go = vi.fn(async (route: string) => {
    pages[at] = route;
  });
  const env: BackEnv = {
    history: {
      get length() { return length; },
      back() {
        if (at === 0) return;
        at -= 1;
        setTimeout(() => events.dispatchEvent(new Event('popstate')), 10);
      },
    },
    location: { get href() { return `http://app${pages[at]}`; } },
    events,
    go,
  };
  return { env, go, here: () => pages[at] };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('leave', () => {
  it('goes back to the page behind', async () => {
    const b = browser(['/messenger', '/messenger/chat?id=dm%3Aa']);
    const left = leave('/messenger', b.env);
    await vi.advanceTimersByTimeAsync(10);
    expect(await left).toBe('back');
    expect(b.here()).toBe('/messenger');
    expect(b.go).not.toHaveBeenCalled();
  });

  it('goes to the fallback when a back went nowhere', async () => {
    // A tap started the app into the chat; a page was opened ahead of it and left.
    const b = browser(['/messenger/chat?id=dm%3Aa', '/messenger/call'], 0);
    expect(b.env.history.length).toBe(2);
    const left = leave('/messenger', b.env);
    await vi.advanceTimersByTimeAsync(BACK_WAIT_MS);
    expect(await left).toBe('fallback');
    expect(b.go).toHaveBeenCalledWith('/messenger');
    expect(b.here()).toBe('/messenger');
  });

  it('goes to the fallback when a back landed on the same page', async () => {
    const b = browser(['/messenger/chat?id=dm%3Aa', '/messenger/chat?id=dm%3Aa']);
    const left = leave(() => '/messenger', b.env);
    await vi.advanceTimersByTimeAsync(BACK_WAIT_MS);
    expect(await left).toBe('fallback');
    expect(b.go).toHaveBeenCalledWith('/messenger');
  });

  it('goes to the fallback at once when the history has one page', async () => {
    const b = browser(['/notes/note?id=1']);
    expect(await leave('/notes', b.env)).toBe('fallback');
    expect(b.go).toHaveBeenCalledWith('/notes');
  });

  it('does nothing for a second tap while the first is under way', async () => {
    const b = browser(['/messenger', '/messenger/chat?id=dm%3Aa', '/messenger/chat?id=dm%3Ab']);
    const first = leave('/messenger', b.env);
    expect(await leave('/messenger', b.env)).toBe('busy');
    await vi.advanceTimersByTimeAsync(10);
    expect(await first).toBe('back');
    expect(b.here()).toBe('/messenger/chat?id=dm%3Aa');
  });
});

describe('leaveOnce', () => {
  it('stays left after a back', async () => {
    const b = browser(['/messenger', '/messenger/call']);
    const out = leaveOnce(() => '/messenger', b.env);
    const first = out();
    await vi.advanceTimersByTimeAsync(10);
    await first;
    expect(b.here()).toBe('/messenger');
    // The end of the call asks again while the page goes: no second back.
    await out();
    expect(b.here()).toBe('/messenger');
    expect(b.go).not.toHaveBeenCalled();
  });

  it('tries again after a back that went nowhere', async () => {
    const b = browser(['/messenger/chat?id=dm%3Aa', '/messenger/call'], 0);
    const out = leaveOnce(() => '/messenger/chat?id=dm%3Aa', b.env);
    const first = out();
    await vi.advanceTimersByTimeAsync(BACK_WAIT_MS);
    await first;
    expect(b.go).toHaveBeenCalledTimes(1);
    // The arrow is still alive.
    const second = out();
    await vi.advanceTimersByTimeAsync(BACK_WAIT_MS);
    await second;
    expect(b.go).toHaveBeenCalledTimes(2);
  });
});

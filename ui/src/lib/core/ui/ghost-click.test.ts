// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AFTER_LIFT_MS, swallowNextClick, type ClickTarget } from './ghost-click';

// A window that only keeps its listeners.
function fakeWindow() {
  const listeners = new Map<string, Set<(e: Event) => void>>();
  const target: ClickTarget = {
    addEventListener: (type, fn) => {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type)!.add(fn);
    },
    removeEventListener: (type, fn) => listeners.get(type)?.delete(fn),
  };
  const event = (type: string, cancelable = true) => ({
    type, cancelable, preventDefault: vi.fn(), stopPropagation: vi.fn(),
  });
  const fire = (type: string, cancelable = true) => {
    const e = event(type, cancelable);
    for (const fn of [...(listeners.get(type) ?? [])]) fn(e as unknown as Event);
    return e;
  };
  const count = () => [...listeners.values()].reduce((n, s) => n + s.size, 0);
  return { target, fire, count };
}

describe('swallowNextClick', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('swallows the click of the lifted finger (iOS) and lets the next one through', () => {
    const w = fakeWindow();
    swallowNextClick(w.target);
    const end = w.fire('touchend');
    expect(end.preventDefault).toHaveBeenCalled();
    const ghost = w.fire('click');
    expect(ghost.preventDefault).toHaveBeenCalled();
    expect(ghost.stopPropagation).toHaveBeenCalled();
    expect(w.count()).toBe(0);
    const next = w.fire('click');
    expect(next.preventDefault).not.toHaveBeenCalled();
  });

  it('waits while the finger rests, however long', () => {
    const w = fakeWindow();
    swallowNextClick(w.target);
    vi.advanceTimersByTime(5000);
    expect(w.count()).toBeGreaterThan(0);
    expect(w.fire('click').preventDefault).toHaveBeenCalled();
  });

  it('ends a moment after the lift when no click comes (Android)', () => {
    const w = fakeWindow();
    swallowNextClick(w.target);
    w.fire('pointerup');
    vi.advanceTimersByTime(AFTER_LIFT_MS);
    expect(w.count()).toBe(0);
    expect(w.fire('click').preventDefault).not.toHaveBeenCalled();
  });
});

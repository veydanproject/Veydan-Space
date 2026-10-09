// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { longpress } from './longpress';

// No DOM in these tests: a node that only keeps its listeners is enough.
function fakeNode() {
  const listeners = new Map<string, (e: unknown) => void>();
  const node = { addEventListener: (type: string, fn: (e: unknown) => void) => listeners.set(type, fn), removeEventListener: () => {} };
  return { node: node as unknown as HTMLElement, fire: (type: string, e: unknown) => listeners.get(type)?.(e) };
}

describe('longpress', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('tells where the finger rested and what it went down on', () => {
    const onpress = vi.fn();
    const { node, fire } = fakeNode();
    longpress(node, { onpress });
    const caption = { id: 'caption' };
    fire('pointerdown', { pointerType: 'touch', clientX: 10, clientY: 20, target: caption });
    vi.advanceTimersByTime(450);
    expect(onpress).toHaveBeenCalledWith({ x: 10, y: 20, target: caption });
  });

  it('leaves the mouse to its right button', () => {
    const onpress = vi.fn();
    const { node, fire } = fakeNode();
    longpress(node, { onpress });
    fire('pointerdown', { pointerType: 'mouse', clientX: 10, clientY: 20, target: null });
    vi.advanceTimersByTime(1000);
    expect(onpress).not.toHaveBeenCalled();
  });
});

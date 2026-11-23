// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { CANCEL_PX, IDLE, LOCK_PX, step, type Gesture, type GestureAction, type GestureEvent } from './record-gesture';

function run(events: GestureEvent[]): { g: Gesture; actions: GestureAction[] } {
  let g = IDLE;
  const actions: GestureAction[] = [];
  for (const e of events) {
    const r = step(g, e);
    g = r.g;
    if (r.action !== 'none') actions.push(r.action);
  }
  return { g, actions };
}

const down: GestureEvent = { type: 'down', x: 300, y: 600 };
const hold: GestureEvent = { type: 'hold' };
const up: GestureEvent = { type: 'up' };

describe('record gesture', () => {
  it('a tap switches the kind', () => {
    const r = run([down, up]);
    expect(r.actions).toEqual(['toggle']);
    expect(r.g.phase).toBe('idle');
  });

  it('a hold records and a lift ends it', () => {
    expect(run([down, hold]).actions).toEqual(['start']);
    expect(run([down, hold, up]).actions).toEqual(['start', 'release']);
  });

  it('a slide to the left discards, and the lift after it does nothing', () => {
    const r = run([down, hold, { type: 'move', x: 300 - CANCEL_PX, y: 600 }, up]);
    expect(r.actions).toEqual(['start', 'discard']);
    expect(r.g.phase).toBe('idle');
  });

  it('a slide up locks, and the lift after it does nothing', () => {
    const r = run([down, hold, { type: 'move', x: 300, y: 600 - LOCK_PX }]);
    expect(r.actions).toEqual(['start', 'lock']);
    expect(r.g.phase).toBe('locked');
    expect(step(r.g, up)).toEqual({ g: IDLE, action: 'none' });
  });

  it('a short slide only moves the button', () => {
    const r = run([down, hold, { type: 'move', x: 260, y: 590 }]);
    expect(r.actions).toEqual(['start']);
    expect(r.g).toMatchObject({ phase: 'holding', dx: -40, dy: 0 });
  });

  it('follows one axis: a diagonal slide neither discards nor locks', () => {
    const r = run([down, hold, { type: 'move', x: 300 - (CANCEL_PX - 1), y: 600 - (CANCEL_PX - 2) }]);
    expect(r.actions).toEqual(['start']);
    expect(r.g.dy).toBe(0);
  });

  it('never goes right or down', () => {
    const r = run([down, hold, { type: 'move', x: 400, y: 700 }]);
    expect(r.g).toMatchObject({ dx: 0, dy: 0 });
  });

  it('a pointer taken by the system discards a recording and forgets a press', () => {
    expect(run([down, hold, { type: 'cancel' }]).actions).toEqual(['start', 'discard']);
    expect(run([down, { type: 'cancel' }]).actions).toEqual([]);
  });

  it('a move before the hold starts nothing', () => {
    expect(run([down, { type: 'move', x: 20, y: 600 }, up]).actions).toEqual(['toggle']);
  });
});

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The record button of the composer, as a state machine without a DOM:
// a tap (on a phone it starts a recording that goes on by itself, as a locked
// one, design 15; a computer switches voice and circle), a hold records, a
// slide to the left discards, a slide up locks the recording so the finger can leave.

/** A press shorter than this is a tap, milliseconds. */
export const HOLD_MS = 220;
/** A slide this far to the left discards the recording, pixels. */
export const CANCEL_PX = 220;
/** A slide this far up locks the recording, pixels. */
export const LOCK_PX = 140;
/** A finger that went this far before the hold is a swipe, not a tap: its lift starts nothing, pixels. */
export const SLOP_PX = 16;

export type GesturePhase = 'idle' | 'pressed' | 'holding' | 'locked';

export interface Gesture {
  phase: GesturePhase;
  x0: number;
  y0: number;
  /** How far the finger went on its main axis; never positive. */
  dx: number;
  dy: number;
}

export type GestureEvent =
  | { type: 'down'; x: number; y: number }
  /** The finger rested for `HOLD_MS`. */
  | { type: 'hold' }
  | { type: 'move'; x: number; y: number }
  | { type: 'up' }
  /** The system took the pointer away. */
  | { type: 'cancel' };

/** What the composer does after a step. */
export type GestureAction = 'none' | 'tap' | 'start' | 'discard' | 'lock' | 'release';

export const IDLE: Gesture = { phase: 'idle', x0: 0, y0: 0, dx: 0, dy: 0 };

export function step(g: Gesture, e: GestureEvent): { g: Gesture; action: GestureAction } {
  const stay = { g, action: 'none' as const };
  switch (g.phase) {
    case 'idle':
      return e.type === 'down' ? { g: { phase: 'pressed', x0: e.x, y0: e.y, dx: 0, dy: 0 }, action: 'none' } : stay;
    case 'pressed':
      if (e.type === 'hold') return { g: { ...g, phase: 'holding' }, action: 'start' };
      if (e.type === 'up') return { g: IDLE, action: 'tap' };
      if (e.type === 'cancel') return { g: IDLE, action: 'none' };
      // A quick swipe off the button (to the left to discard) must not turn into a tap that records.
      if (e.type === 'move' && Math.hypot(e.x - g.x0, e.y - g.y0) > SLOP_PX) return { g: { ...IDLE, phase: 'locked' }, action: 'none' };
      return stay;
    case 'holding': {
      if (e.type === 'up') return { g: IDLE, action: 'release' };
      if (e.type === 'cancel') return { g: IDLE, action: 'discard' };
      if (e.type !== 'move') return stay;
      let dx = Math.min(0, e.x - g.x0);
      let dy = Math.min(0, e.y - g.y0);
      // One axis at a time: a diagonal slide is neither a discard nor a lock.
      if (Math.abs(dx) >= Math.abs(dy)) dy = 0; else dx = 0;
      if (dx <= -CANCEL_PX) return { g: { ...IDLE, phase: 'locked' }, action: 'discard' };
      if (dy <= -LOCK_PX) return { g: { ...IDLE, phase: 'locked' }, action: 'lock' };
      return { g: { ...g, dx, dy }, action: 'none' };
    }
    case 'locked':
      // The finger that locked, discarded or swiped is still down: its lift does nothing.
      return e.type === 'up' || e.type === 'cancel' ? { g: IDLE, action: 'none' } : stay;
  }
}

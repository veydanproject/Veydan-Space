// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The arithmetic of a pane whose width is dragged (PaneResizer): bounds,
// a step of the keyboard, and the width under the pointer.

/** One press of ←/→ on a focused resizer moves the edge so far. */
export const KEY_STEP = 16;

/** `px` within `min…max`, whole; a `max` below `min` gives `min`. */
export function clampWidth(px: number, min: number, max: number): number {
  const top = Math.max(min, max);
  if (!Number.isFinite(px)) return min;
  return Math.round(Math.min(top, Math.max(min, px)));
}

/**
 * The widest the side column of a pane `paneWidth` wide may be: `cap`, and
 * never more than `share` of the pane, never less than `min`.
 */
export function sideMax(paneWidth: number, min: number, cap: number, share = 0.6): number {
  if (!(paneWidth > 0)) return cap;
  return Math.max(min, Math.min(cap, Math.floor(paneWidth * share)));
}

/**
 * The width after a drag from `startX` (the pointer's x when it went
 * down, at width `startWidth`) to `x`. A pane right of the edge grows as
 * the edge goes left; one left of it, as it goes right.
 */
export function dragWidth(startWidth: number, startX: number, x: number, side: 'left' | 'right'): number {
  return side === 'right' ? startWidth + (startX - x) : startWidth + (x - startX);
}

/**
 * The width after a key on a focused resizer, or `null` when the key does
 * nothing. ← widens a pane on the right and narrows one on the left; Home
 * and End go to the bounds.
 */
export function keyWidth(width: number, key: string, side: 'left' | 'right', min: number, max: number): number | null {
  const grow = side === 'right' ? 'ArrowLeft' : 'ArrowRight';
  const shrink = side === 'right' ? 'ArrowRight' : 'ArrowLeft';
  if (key === grow) return clampWidth(width + KEY_STEP, min, max);
  if (key === shrink) return clampWidth(width - KEY_STEP, min, max);
  if (key === 'Home') return clampWidth(min, min, max);
  if (key === 'End') return clampWidth(max, min, max);
  return null;
}

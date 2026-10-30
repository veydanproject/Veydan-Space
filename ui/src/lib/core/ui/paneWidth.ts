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

// The list's column of the messenger (chats, groups, channels, calls,
// contacts): words from 260 to 480 px, 350 by default; dragged under 180 it
// is the column of covers, 72 px, and goes no narrower; dragged out of it
// past 180 it comes back at 260.

/** The column of covers. */
export const LIST_COVERS = 72;
/** The narrowest and the widest with words, and the default. */
export const LIST_MIN = 260;
export const LIST_MAX = 480;
export const LIST_DEFAULT = 350;
/** Under this the column is its covers. */
export const LIST_FOLD = 180;
/** What the runtime keeps for the covers: it keeps 200…2000 px (`ui_prefs`), so the covers go as its least. */
export const LIST_KEPT_COVERS = 200;
/** The least the conversation beside the list keeps of the window. */
const FEED_MIN = 400;

/** The width under the pointer (`px`, already within the bounds) → the one the column takes. */
export function listWidth(px: number): number {
  if (px < LIST_FOLD) return LIST_COVERS;
  if (px < LIST_MIN) return LIST_MIN;
  return px;
}

/** The keyboard (PaneResizer's `snap`): a step out of the covers is 260 at once, a step under 260 the covers. */
export function listKeyWidth(px: number, prev: number): number {
  if (px > prev && prev < LIST_MIN) return LIST_MIN;
  if (px < prev && px < LIST_MIN) return LIST_COVERS;
  return listWidth(px);
}

/** The widest the column may be in a shell `shellWidth` wide (its sections' column and gaps taken): 480, the conversation keeping 400. */
export function listMax(shellWidth: number): number {
  if (!(shellWidth > 0)) return LIST_MAX;
  return Math.max(LIST_MIN, Math.min(LIST_MAX, Math.floor(shellWidth - 72 - 24 - FEED_MIN)));
}

/** The width kept (`null`: never set) → the column's: anything kept under 260 is the covers. */
export function keptListWidth(kept: number | null, max: number): number {
  if (kept == null) return clampWidth(LIST_DEFAULT, LIST_MIN, max);
  return kept < LIST_MIN ? LIST_COVERS : clampWidth(kept, LIST_MIN, max);
}

/** The column's width → what is kept. */
export function keepListWidth(px: number): number {
  return px < LIST_MIN ? LIST_KEPT_COVERS : Math.round(px);
}

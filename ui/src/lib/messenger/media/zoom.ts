// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// How the viewer zooms a picture: a scale over the picture as it fits the
// screen (1 = fits, 100 %) and a shift of its centre from the centre of the
// screen. Points are taken from the centre of the screen.

export interface View {
  scale: number;
  x: number;
  y: number;
}

export interface Size {
  w: number;
  h: number;
}

/** The picture as it fits the screen. */
export const FIT: View = { scale: 1, x: 0, y: 0 };
export const ZOOM_MAX = 8;
/** What the buttons step through and the menu offers. */
export const ZOOM_STEPS = [1, 1.5, 2, 3, 4, 6, 8];

/** `+ 0` turns a -0 (an empty margin of -0) into 0. */
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v)) + 0;

/**
 * Within reason: no smaller than fitting, no larger than `ZOOM_MAX`, and the
 * picture never leaves an empty edge where it could fill the screen.
 */
export function clampView(v: View, pic: Size, screen: Size): View {
  const scale = clamp(v.scale, 1, ZOOM_MAX);
  const mx = Math.max(0, (pic.w * scale - screen.w) / 2);
  const my = Math.max(0, (pic.h * scale - screen.h) / 2);
  return { scale, x: clamp(v.x, -mx, mx), y: clamp(v.y, -my, my) };
}

/**
 * The view at `scale` that puts the spot of the picture under `from` (in
 * `v`) under `to`: a wheel or a button zooms about one point (`to` = `from`),
 * two fingers carry it as they move.
 */
export function anchored(v: View, scale: number, from: { x: number; y: number }, to: { x: number; y: number }, pic: Size, screen: Size): View {
  const s = clamp(scale, 1, ZOOM_MAX);
  const qx = (from.x - v.x) / v.scale;
  const qy = (from.y - v.y) / v.scale;
  return clampView({ scale: s, x: to.x - s * qx, y: to.y - s * qy }, pic, screen);
}

/** The next step up (1) or down (-1) from `scale`. */
export function stepped(scale: number, dir: 1 | -1): number {
  if (dir > 0) return ZOOM_STEPS.find((s) => s > scale + 0.01) ?? ZOOM_MAX;
  return [...ZOOM_STEPS].reverse().find((s) => s < scale - 0.01) ?? 1;
}

/** The scale as the viewer names it. */
export const percent = (scale: number) => `${Math.round(scale * 100)}%`;

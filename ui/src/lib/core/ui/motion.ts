// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The ways in and out of the layers (design kit, MOTION_SPEC), as Svelte
// transitions: `in:sheetIn out:sheetOut`. Reduced motion: none of them moves.
//
//   M02 dialog   in 200 ms from 8 below at .98, out 150 ms; backdrop 200 / 150
//   M03 drawer   in 280 ms from its side, out 200 ms
//   M04 sheet    in 280 ms from below, out 200 ms (from where the finger left it)
//   M05 menu     in 150 ms from .96 at the point, out 100 ms

import { cubicOut } from 'svelte/easing';

const still = () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
const ms = (n: number) => (still() ? 0 : n);
/** --ease-drawer, cubic-bezier(0.2, 0.7, 0.2, 1), close enough. */
const drawerEase = cubicOut;

export function backdropIn(_node: Element) {
  return { duration: ms(200), css: (t: number) => `opacity:${t}` };
}
export function backdropOut(_node: Element) {
  return { duration: ms(150), css: (t: number) => `opacity:${t}` };
}

export function dialogIn(_node: Element) {
  return { duration: ms(200), easing: cubicOut, css: (t: number) => `opacity:${t};transform:translateY(${(1 - t) * 8}px) scale(${0.98 + 0.02 * t})` };
}
export function dialogOut(_node: Element) {
  return { duration: ms(150), css: (t: number) => `opacity:${t};transform:translateY(${(1 - t) * 8}px) scale(${0.98 + 0.02 * t})` };
}

export function menuOut(_node: Element) {
  return { duration: ms(100), css: (t: number) => `opacity:${t};transform:scale(${0.96 + 0.04 * t})` };
}

export function sheetIn(_node: Element) {
  return { duration: ms(280), easing: drawerEase, css: (t: number) => `transform:translateY(${(1 - t) * 100}%)` };
}
/** From where it stands (a pull of the finger leaves it lower) down out of sight. */
export function sheetOut(node: Element) {
  const from = new DOMMatrix(getComputedStyle(node).transform).m42 || 0;
  return { duration: ms(200), css: (t: number) => `transform:translateY(calc(${from * t}px + ${(1 - t) * 100}%))` };
}

export function drawerIn(_node: Element, { side = 'right' }: { side?: 'left' | 'right' } = {}) {
  const dir = side === 'right' ? 1 : -1;
  return { duration: ms(280), easing: drawerEase, css: (t: number) => `opacity:${0.4 + 0.6 * t};transform:translateX(${(1 - t) * 30 * dir}px)` };
}
export function drawerOut(_node: Element, { side = 'right' }: { side?: 'left' | 'right' } = {}) {
  const dir = side === 'right' ? 1 : -1;
  return { duration: ms(200), css: (t: number) => `opacity:${t};transform:translateX(${(1 - t) * 30 * dir}px)` };
}

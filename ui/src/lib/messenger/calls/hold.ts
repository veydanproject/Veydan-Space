// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The "other" action of a button: a finger or the mouse resting on it, or a
// right click. Unlike shared/longpress.ts (a finger only: the mouse has its
// right button there), the mouse's hold counts too: a call button's other
// action is one people look for by holding it. The click that ends a hold
// does not also press the button.

export interface HoldOptions {
  onhold: () => void;
  /** Milliseconds it must rest. */
  delay?: number;
}

const MOVE_TOLERANCE = 10;
export const HOLD_MS = 550;

export function hold(node: HTMLElement, options: HoldOptions) {
  let opts = options;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let start = { x: 0, y: 0 };
  let fired = false;

  const clear = () => {
    if (timer) clearTimeout(timer);
    timer = null;
  };

  const fire = () => {
    fired = true;
    navigator.vibrate?.(12);
    opts.onhold();
  };

  const down = (e: PointerEvent) => {
    if (e.button !== 0) return;
    fired = false;
    start = { x: e.clientX, y: e.clientY };
    clear();
    timer = setTimeout(() => {
      timer = null;
      fire();
    }, opts.delay ?? HOLD_MS);
  };

  const move = (e: PointerEvent) => {
    if (!timer) return;
    if (Math.abs(e.clientX - start.x) > MOVE_TOLERANCE || Math.abs(e.clientY - start.y) > MOVE_TOLERANCE) clear();
  };

  const click = (e: MouseEvent) => {
    if (!fired) return;
    fired = false;
    e.preventDefault();
    e.stopImmediatePropagation();
  };

  // A right click; Android also raises it on a long press, which ran already.
  const context = (e: MouseEvent) => {
    e.preventDefault();
    if (fired || timer) { clear(); if (!fired) fire(); return; }
    fire();
    // No click follows a right click.
    fired = false;
  };

  node.addEventListener('pointerdown', down);
  node.addEventListener('pointermove', move);
  node.addEventListener('pointerup', clear);
  node.addEventListener('pointercancel', clear);
  node.addEventListener('pointerleave', clear);
  node.addEventListener('click', click, true);
  node.addEventListener('contextmenu', context);

  return {
    update(next: HoldOptions) { opts = next; },
    destroy() {
      clear();
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerup', clear);
      node.removeEventListener('pointercancel', clear);
      node.removeEventListener('pointerleave', clear);
      node.removeEventListener('click', click, true);
      node.removeEventListener('contextmenu', context);
    },
  };
}

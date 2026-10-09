// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Long press for touch screens: the counterpart of a right click. Fires
// once after the finger rested on the element; moving or lifting cancels.

export interface LongPressOptions {
  /** `target` is what the finger went down on: a shared parent finds the part pressed. */
  onpress: (point: { x: number; y: number; target: EventTarget | null }) => void;
  /** Milliseconds the finger must rest. */
  delay?: number;
}

const MOVE_TOLERANCE = 10;

export function longpress(node: HTMLElement, options: LongPressOptions) {
  let opts = options;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let start = { x: 0, y: 0 };
  let target: EventTarget | null = null;
  let fired = false;

  const clear = () => {
    if (timer) clearTimeout(timer);
    timer = null;
  };

  const down = (e: PointerEvent) => {
    if (e.pointerType === 'mouse') return; // the mouse has a right button
    fired = false;
    start = { x: e.clientX, y: e.clientY };
    target = e.target;
    clear();
    timer = setTimeout(() => {
      timer = null;
      fired = true;
      navigator.vibrate?.(12);
      opts.onpress({ x: start.x, y: start.y, target });
    }, opts.delay ?? 450);
  };

  const move = (e: PointerEvent) => {
    if (!timer) return;
    if (Math.abs(e.clientX - start.x) > MOVE_TOLERANCE || Math.abs(e.clientY - start.y) > MOVE_TOLERANCE) clear();
  };

  // The tap that ends a long press must not also act as a click.
  const click = (e: MouseEvent) => {
    if (!fired) return;
    fired = false;
    e.preventDefault();
    e.stopPropagation();
  };

  // Android also raises `contextmenu` on a long press: ours already ran.
  const context = (e: Event) => {
    if (fired) e.preventDefault();
  };

  node.addEventListener('pointerdown', down);
  node.addEventListener('pointermove', move);
  node.addEventListener('pointerup', clear);
  node.addEventListener('pointercancel', clear);
  node.addEventListener('pointerleave', clear);
  node.addEventListener('click', click, true);
  node.addEventListener('contextmenu', context);

  return {
    update(next: LongPressOptions) { opts = next; },
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

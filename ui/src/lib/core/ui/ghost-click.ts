// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The click of the finger that ends a long press. Android raises none after a
// long press; WebKit on iOS raises one when the finger lifts, aimed at what is
// under the finger by then — the sheet or the menu the long press just opened —
// so its item under the finger fired by itself. A long press calls this when
// it fires: the next click anywhere is swallowed, and the lift itself is
// cancelled so WebKit makes no click of it. The guard ends a moment after the
// finger lifts, whether a click came or not.

/** What the guard listens on: `window` in the app, a stand-in in the tests. */
export interface ClickTarget {
  addEventListener(type: string, fn: (e: Event) => void, options?: AddEventListenerOptions | boolean): void;
  removeEventListener(type: string, fn: (e: Event) => void, options?: EventListenerOptions | boolean): void;
}

/** How long after the finger lifts a click still counts as the lift's own. */
export const AFTER_LIFT_MS = 400;

export function swallowNextClick(
  target: ClickTarget | undefined = typeof window === 'undefined' ? undefined : window,
): () => void {
  if (!target) return () => {};
  let done = false;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const click = (e: Event) => {
    e.preventDefault();
    e.stopPropagation();
    finish();
  };
  const lift = (e: Event) => {
    if (e.type === 'touchend' && e.cancelable) e.preventDefault();
    if (!timer) timer = setTimeout(finish, AFTER_LIFT_MS);
  };
  const finish = () => {
    if (done) return;
    done = true;
    if (timer) clearTimeout(timer);
    target.removeEventListener('click', click, true);
    target.removeEventListener('touchend', lift, true);
    target.removeEventListener('pointerup', lift, true);
    target.removeEventListener('pointercancel', lift, true);
  };

  target.addEventListener('click', click, true);
  target.addEventListener('touchend', lift, { capture: true, passive: false });
  target.addEventListener('pointerup', lift, true);
  target.addEventListener('pointercancel', lift, true);
  return finish;
}

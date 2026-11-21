// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The iPhone keyboard has no key that puts it away (Android's has one, and
// the bar above it with «Done» is off in our webview). Messengers on the
// iPhone let it go when the reader drags the conversation down or taps it;
// this does the same for the element it is put on. Nothing happens off iOS.

import { platform } from '$lib/core/platform';

/** How far down the finger must go before the drag counts. */
export const DRAG_PX = 24;
/** What still counts as a tap rather than a drag. */
export const TAP_PX = 10;

/** A tap on these does what they do; the keyboard stays for them. */
const ACTIVE = 'a, button, input, textarea, select, label, video, audio, [role="button"], [contenteditable]:not([contenteditable="false"])';

/** The decision alone, for the tests: what one touch from start to now asks. */
export function dismissFor(dx: number, dy: number, ended: boolean, onActive: boolean): boolean {
  if (dy > DRAG_PX && dy > Math.abs(dx)) return true; // dragged down
  return ended && !onActive && Math.abs(dx) < TAP_PX && Math.abs(dy) < TAP_PX; // tapped
}

function typingField(): HTMLElement | null {
  const a = document.activeElement;
  return a instanceof HTMLElement && a.matches('input, textarea, [contenteditable]:not([contenteditable="false"])') ? a : null;
}

/** Svelte action: a drag down or a tap on the node lets the keyboard go (iOS). */
export function dismissKeyboard(node: HTMLElement) {
  if (platform !== 'ios') return {};
  let x0 = 0;
  let y0 = 0;
  let onActive = false;
  let done = false;

  const start = (e: TouchEvent) => {
    const t = e.touches[0];
    x0 = t.clientX;
    y0 = t.clientY;
    done = false;
    onActive = e.target instanceof Element && !!e.target.closest(ACTIVE);
  };
  const check = (e: TouchEvent, ended: boolean) => {
    if (done) return;
    const t = ended ? e.changedTouches[0] : e.touches[0];
    if (!t || !dismissFor(t.clientX - x0, t.clientY - y0, ended, onActive)) return;
    done = true;
    typingField()?.blur();
  };
  const move = (e: TouchEvent) => check(e, false);
  const end = (e: TouchEvent) => check(e, true);

  node.addEventListener('touchstart', start, { passive: true });
  node.addEventListener('touchmove', move, { passive: true });
  node.addEventListener('touchend', end, { passive: true });
  return {
    destroy() {
      node.removeEventListener('touchstart', start);
      node.removeEventListener('touchmove', move);
      node.removeEventListener('touchend', end);
    },
  };
}

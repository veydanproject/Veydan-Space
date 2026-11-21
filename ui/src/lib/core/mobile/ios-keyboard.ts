// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The keyboard on iOS, as the Android host gives it: `--kb` on <html> and
// `veydan-keyboard` on window (messenger/shared/keyboard.ts), so every screen
// that lifts itself above the keyboard on Android does on the iPhone too.
//
// WKWebView neither resizes the page nor says where the keyboard is: it
// shrinks the visual viewport and scrolls the whole document up to show the
// focused field, so the header went off the top and the page did not know
// the keyboard was there (innerHeight shrinks with it, the usual sum is 0).
// The keyboard is the height the visual viewport lost; the document is held
// at the top, the page lifts its own fields by `--kb`.

import { platform } from '$lib/core/platform';

/** The fields a tap types into (not the boxes, buttons and pickers an input can be). */
const TYPED = 'textarea, input:not([type=checkbox]):not([type=radio]):not([type=button]):not([type=submit]):not([type=file]):not([type=range]):not([type=color])';
/** What still counts as a tap rather than a drag. */
const TAP_PX = 10;

/** Starts on iOS; returns the stop function (a no-op elsewhere). */
export function startIosKeyboard(): () => void {
  const vv = window.visualViewport;
  if (platform !== 'ios' || !vv) return () => {};
  // The height with no keyboard: the largest the visual viewport has been.
  let full = Math.max(vv.height, window.innerHeight);
  let last = -1;
  const root = document.documentElement;
  const tick = () => {
    if (vv.height > full) full = vv.height;
    const kb = Math.max(0, Math.round(full - vv.height));
    if (window.scrollY || vv.offsetTop) window.scrollTo(0, 0);
    if (kb === last) return;
    last = kb;
    root.style.setProperty('--kb', `${kb}px`);
    window.dispatchEvent(new Event('veydan-keyboard'));
  };
  // A turn of the phone changes the height with no keyboard.
  const turned = () => {
    full = 0;
    tick();
  };
  // A tap on a field: WebKit focuses it and scrolls the whole document to
  // show it, `tick` scrolls it back, and the screen played a scroll up and
  // down each time the keyboard came. The tap focuses the field itself,
  // without the scroll; the keyboard still comes, the tap being the user's.
  let x0 = 0;
  let y0 = 0;
  const down = (e: TouchEvent) => {
    x0 = e.touches[0].clientX;
    y0 = e.touches[0].clientY;
  };
  const up = (e: TouchEvent) => {
    const field = e.target instanceof Element ? e.target.closest<HTMLElement>(TYPED) : null;
    if (!field || document.activeElement === field) return;
    const t = e.changedTouches[0];
    if (!t || Math.abs(t.clientX - x0) > TAP_PX || Math.abs(t.clientY - y0) > TAP_PX) return;
    e.preventDefault();
    field.focus({ preventScroll: true });
  };
  vv.addEventListener('resize', tick);
  vv.addEventListener('scroll', tick);
  window.addEventListener('orientationchange', turned);
  window.addEventListener('touchstart', down, { capture: true, passive: true });
  window.addEventListener('touchend', up, { capture: true, passive: false });
  tick();
  return () => {
    vv.removeEventListener('resize', tick);
    vv.removeEventListener('scroll', tick);
    window.removeEventListener('orientationchange', turned);
    window.removeEventListener('touchstart', down, true);
    window.removeEventListener('touchend', up, true);
    root.style.removeProperty('--kb');
  };
}

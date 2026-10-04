// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Svelte attachment: fires `cb` after a 500ms press without movement and
// swallows the click that follows.

import type { Attachment } from 'svelte/attachments';

export function longpress(cb: () => void): Attachment<HTMLElement> {
  return (el) => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let fired = false;
    let x = 0;
    let y = 0;

    const cancel = () => {
      clearTimeout(timer);
      timer = undefined;
    };
    const start = (e: TouchEvent) => {
      fired = false;
      x = e.touches[0].clientX;
      y = e.touches[0].clientY;
      timer = setTimeout(() => {
        fired = true;
        cb();
      }, 500);
    };
    const move = (e: TouchEvent) => {
      if (Math.abs(e.touches[0].clientX - x) > 10 || Math.abs(e.touches[0].clientY - y) > 10) cancel();
    };
    const click = (e: Event) => {
      if (fired) {
        e.preventDefault();
        e.stopImmediatePropagation();
        fired = false;
      }
    };
    const ctx = (e: Event) => e.preventDefault();

    el.addEventListener('touchstart', start, { passive: true });
    el.addEventListener('touchmove', move, { passive: true });
    el.addEventListener('touchend', cancel);
    el.addEventListener('touchcancel', cancel);
    el.addEventListener('click', click, true);
    el.addEventListener('contextmenu', ctx);
    return () => {
      cancel();
      el.removeEventListener('touchstart', start);
      el.removeEventListener('touchmove', move);
      el.removeEventListener('touchend', cancel);
      el.removeEventListener('touchcancel', cancel);
      el.removeEventListener('click', click, true);
      el.removeEventListener('contextmenu', ctx);
    };
  };
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A shown secret hides itself (design kit, secret field): 30 s after it was
// revealed it is masked again, and the caption under it counts the seconds
// down, «Скроется через N с». Hiding it by hand, locking the app or leaving
// the page stops the count.
//
//   const hide = new AutoHide(() => (revealed = ''));
//   revealed = value; hide.start();      // on reveal
//   revealed = ''; hide.stop();          // on hide
//   {#if hide.left}…{hide.left}…{/if}    // the caption

export const AUTO_HIDE_SECONDS = 30;

export class AutoHide {
  /** Seconds before the secret hides; 0 while nothing is counting. */
  left = $state(0);
  #timer: ReturnType<typeof setInterval> | undefined;
  #onhide: () => void;

  constructor(onhide: () => void) {
    this.#onhide = onhide;
  }

  start(seconds = AUTO_HIDE_SECONDS) {
    this.stop();
    this.left = seconds;
    const until = Date.now() + seconds * 1000;
    this.#timer = setInterval(() => {
      this.left = Math.max(0, Math.ceil((until - Date.now()) / 1000));
      if (this.left === 0) {
        this.stop();
        this.#onhide();
      }
    }, 250);
  }

  stop() {
    clearInterval(this.#timer);
    this.#timer = undefined;
    this.left = 0;
  }
}

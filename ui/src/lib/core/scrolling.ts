// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** How long the thumb stays after the last scroll event. */
const LINGER_MS = 800;

/**
 * Whether an element is scrolling now: `data-scrolling` on it from a scroll
 * until LINGER_MS after the last one. A touch screen has no hover, so its
 * scrollbar thumb (base.css) asks for this. One listener for the whole
 * document: `scroll` does not bubble, it is caught on the way down.
 * Returns what takes the listener off.
 */
export function installScrolling(): () => void {
  const timers = new Map<Element, ReturnType<typeof setTimeout>>();
  const scroll = (e: Event) => {
    const el = e.target === document ? document.scrollingElement : e.target;
    if (!(el instanceof Element)) return;
    const was = timers.get(el);
    if (was === undefined) el.setAttribute('data-scrolling', '');
    else clearTimeout(was);
    timers.set(
      el,
      setTimeout(() => {
        timers.delete(el);
        el.removeAttribute('data-scrolling');
      }, LINGER_MS),
    );
  };
  document.addEventListener('scroll', scroll, { capture: true, passive: true });
  return () => {
    document.removeEventListener('scroll', scroll, { capture: true });
    for (const [el, timer] of timers) {
      clearTimeout(timer);
      el.removeAttribute('data-scrolling');
    }
    timers.clear();
  };
}

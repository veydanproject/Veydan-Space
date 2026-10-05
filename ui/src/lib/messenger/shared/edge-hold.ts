// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A control at the edge of the screen that is held and dragged inward: on
// Android the system takes such a drag for its own "back". The element
// reserves the strip from itself to the right edge through the host's
// `VeydanChrome.setGestureExclusion` (CSS pixels); elsewhere this does nothing.

interface Bridge {
  setGestureExclusion?: (left: number, top: number, width: number, height: number) => void;
}

const PAD = 8;

export function edgeHold(node: HTMLElement) {
  const bridge = (window as Window & { VeydanChrome?: Bridge }).VeydanChrome;
  if (!bridge?.setGestureExclusion) return {};

  const report = () => {
    const r = node.getBoundingClientRect();
    bridge.setGestureExclusion!(r.left - PAD, r.top - PAD, window.innerWidth - r.left + PAD, r.height + 2 * PAD);
  };
  // The keyboard and a growing field move the element.
  const watch = new ResizeObserver(report);
  watch.observe(document.documentElement);
  if (node.parentElement) watch.observe(node.parentElement);
  window.addEventListener('veydan-keyboard', report);
  window.addEventListener('resize', report);
  report();

  return {
    destroy() {
      watch.disconnect();
      window.removeEventListener('veydan-keyboard', report);
      window.removeEventListener('resize', report);
      bridge.setGestureExclusion!(0, 0, 0, 0);
    },
  };
}

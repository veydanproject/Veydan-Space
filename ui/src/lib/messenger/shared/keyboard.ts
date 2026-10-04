// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Height of the on-screen keyboard over the page.
//
// Two sources, the larger wins:
// - `visualViewport`: browsers and webviews that shrink the visual viewport;
// - the host contract for edge-to-edge Android, where the keyboard is drawn
//   over the webview and nothing resizes: the host keeps the inset in the
//   CSS variable `--kb` on <html> and fires `veydan-keyboard` on window
//   when it changes. A host without that contract simply never sets it.

function hostInset(): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue('--kb').trim();
  const n = parseFloat(raw);
  return Number.isFinite(n) ? Math.max(0, Math.round(n)) : 0;
}

export function keyboardInset(): number {
  const vv = window.visualViewport;
  const fromViewport = vv ? Math.max(0, Math.round(window.innerHeight - vv.height - vv.offsetTop)) : 0;
  return Math.max(fromViewport, hostInset());
}

/** Calls `fn` now and on every change; returns the unsubscribe function. */
export function onKeyboard(fn: (inset: number) => void): () => void {
  const vv = window.visualViewport;
  const tick = () => fn(keyboardInset());
  vv?.addEventListener('resize', tick);
  vv?.addEventListener('scroll', tick);
  window.addEventListener('resize', tick);
  window.addEventListener('veydan-keyboard', tick);
  tick();
  return () => {
    vv?.removeEventListener('resize', tick);
    vv?.removeEventListener('scroll', tick);
    window.removeEventListener('resize', tick);
    window.removeEventListener('veydan-keyboard', tick);
  };
}

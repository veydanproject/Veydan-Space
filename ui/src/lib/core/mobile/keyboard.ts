// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** CSS px from the Android IME inset (`--kb`). 0 when the keyboard is closed. */
function nativeKeyboard(): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue('--kb').trim();
  const n = parseFloat(raw);
  return Number.isFinite(n) ? Math.max(0, Math.round(n)) : 0;
}

/** Pixels the IME covers at the bottom of the layout viewport. */
export function keyboardBottom(): number {
  const vv = window.visualViewport;
  const fromVv = vv ? Math.max(0, Math.round(window.innerHeight - vv.height - vv.offsetTop)) : 0;
  // Edge-to-edge Android overlays the keyboard and visualViewport stays 0.
  return Math.max(fromVv, nativeKeyboard());
}

/** Subscribe to IME height changes. Calls `fn` immediately. */
export function onKeyboard(fn: (bottom: number) => void): () => void {
  const vv = window.visualViewport;
  const tick = () => fn(keyboardBottom());
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

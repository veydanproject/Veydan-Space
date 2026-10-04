// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** Hide overlay scrollbars under drawers/modals (Chromium paints them above fixed overlays). */

let locks = 0;
let prevOverflow = '';

function contentEl(): HTMLElement | null {
  return document.querySelector('main.content');
}

export function acquireScrollLock() {
  if (typeof document === 'undefined') return;
  const el = contentEl();
  if (!el) return;
  if (locks === 0) {
    prevOverflow = el.style.overflow;
    el.style.overflow = 'hidden';
  }
  locks += 1;
}

export function releaseScrollLock() {
  if (typeof document === 'undefined') return;
  if (locks === 0) return;
  locks -= 1;
  if (locks > 0) return;
  const el = contentEl();
  if (el) el.style.overflow = prevOverflow;
  prevOverflow = '';
}

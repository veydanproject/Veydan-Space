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

/** The fields a tap types into. */
const TYPED = 'textarea, input:not([type=checkbox]):not([type=radio]):not([type=button]):not([type=submit]):not([type=file]):not([type=range]):not([type=color]), [contenteditable=true]';
/** Air kept between the field in focus and the bottom of what shows it. */
const FIELD_GAP = 8;

/**
 * The keyboard came up over a page's field: the WebView does not know it is there (the host lays it over the
 * page), so nothing scrolled the field into sight. Its scrolling body in the page (.m-page, a first start's
 * step: lifted by --kb)
 * brings it up to its bottom edge, above the page's foot, and above a foot that sticks to that edge inside the
 * body (a form's own .page-foot, the Cloud's «Удалить аккаунт»), with the field's label in sight. A field outside
 * a scrolling body stays as it is.
 */
export function revealFocused(): void {
  const el = document.activeElement;
  if (!(el instanceof HTMLElement) || !el.matches(TYPED)) return;
  const pageEl = el.closest('.m-page, .ob.m');
  let box = el.parentElement;
  while (box && box !== pageEl && !/(auto|scroll)/.test(getComputedStyle(box).overflowY)) box = box.parentElement;
  if (!box || box === pageEl || !pageEl) return;
  const field = el.getBoundingClientRect();
  const view = box.getBoundingClientRect();
  const top = (el.closest('.form-group') ?? el).getBoundingClientRect().top;
  let bottom = view.bottom;
  for (const foot of box.querySelectorAll<HTMLElement>('.page-foot')) {
    const after = el.compareDocumentPosition(foot) & Node.DOCUMENT_POSITION_FOLLOWING;
    // Where the foot stands now, stuck over the field, not what its height says: its margins and the body's
    // padding move the edge it sticks to.
    if (after && getComputedStyle(foot).position === 'sticky') bottom = Math.min(bottom, foot.getBoundingClientRect().top);
  }
  if (field.bottom > bottom - FIELD_GAP) box.scrollTop += field.bottom - (bottom - FIELD_GAP);
  else if (top < view.top) box.scrollTop -= view.top - top;
}

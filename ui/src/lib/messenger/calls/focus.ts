// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Tab inside a modal card: the keyboard goes round its buttons and never
// to the page behind the backdrop. Pure, so it is tested without a page.

/**
 * Where Tab (Shift+Tab with `back`) goes from the button at `at` of
 * `count` (`-1`: none of them, the card itself has the focus): the index
 * to move to when the browser's own step would leave the card, `null` when
 * that step stays inside it. With no button, `-1`: the focus stays.
 */
export function wrapTab(at: number, count: number, back: boolean): number | null {
  if (count === 0) return -1;
  if (at < 0) return back ? count - 1 : 0;
  if (!back && at === count - 1) return 0;
  if (back && at === 0) return count - 1;
  return null;
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Where the call's capsule on a computer may be dragged. Pure, so the
// bounds are tested without a window.

/** How close to the edge of the window the capsule may go. */
export const PANEL_MARGIN = 8;

/**
 * The shift (`translate`) of the capsule held so that the whole capsule
 * stays in the window. Its place without a shift is the middle of the
 * window's width, `top` from the top. A place kept from a larger window, or
 * a window made smaller, would leave it (with End and Mute, the only ones
 * of an audio call) out of reach. A capsule larger than the window keeps
 * its top left in it.
 */
export function clampPanelShift(
  shift: { dx: number; dy: number },
  card: { width: number; height: number; top: number },
  page: { width: number; height: number },
): { dx: number; dy: number } {
  const axis = (d: number, base: number, size: number, room: number) => {
    const at = Math.max(PANEL_MARGIN, Math.min(base + d, room - PANEL_MARGIN - size));
    return at - base;
  };
  return {
    dx: axis(shift.dx, (page.width - card.width) / 2, card.width, page.width),
    dy: axis(shift.dy, card.top, card.height, page.height),
  };
}

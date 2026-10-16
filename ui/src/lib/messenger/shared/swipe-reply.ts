// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Swipe to reply on a touch screen (design kit, M18): the bubble follows
// the finger to the left, up to 64 px; past 48 px a reply arrow shows and
// the phone ticks; letting go there answers the message. A gesture steeper
// than 30° is the list scrolling; the mouse never swipes; a start at the
// screen's edge is the system's back gesture.

export interface SwipeReplyOptions {
  /** Absent: the message cannot be answered, the bubble does not move. */
  onreply?: () => void;
}

const MAX = 64;
const FIRE = 48;
const EDGE = 24;
const SLOP = 8;
const TAN30 = Math.tan(Math.PI / 6);

export function swipeReply(node: HTMLElement, options: SwipeReplyOptions) {
  let opts = options;
  let start: { x: number; y: number } | null = null;
  let swiping = false;
  let armed = false;
  let arrow: HTMLElement | null = null;

  const moved = () => node.querySelector<HTMLElement>('.bubble') ?? node;

  const show = (dx: number) => {
    const el = moved();
    el.style.transition = 'none';
    el.style.transform = `translateX(${dx}px)`;
    if (!arrow) {
      arrow = document.createElement('span');
      arrow.className = 'swipe-arrow';
      arrow.innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M9 14l-4 -4l4 -4"/><path d="M5 10h11a4 4 0 1 1 0 8h-1"/></svg>';
      node.appendChild(arrow);
    }
    const on = -dx >= FIRE;
    arrow.classList.toggle('on', on);
    if (on && !armed) navigator.vibrate?.(12);
    armed = on;
  };

  const reset = () => {
    const el = moved();
    el.style.transition = 'transform var(--dur-base) var(--ease-drawer)';
    el.style.transform = '';
    arrow?.remove();
    arrow = null;
    swiping = false;
    armed = false;
    start = null;
  };

  const down = (e: PointerEvent) => {
    if (e.pointerType === 'mouse' || !opts.onreply) return;
    if (e.clientX < EDGE || e.clientX > window.innerWidth - EDGE) return;
    start = { x: e.clientX, y: e.clientY };
    swiping = false;
  };

  const move = (e: PointerEvent) => {
    if (!start) return;
    const dx = e.clientX - start.x;
    const dy = e.clientY - start.y;
    if (!swiping) {
      if (Math.abs(dx) < SLOP && Math.abs(dy) < SLOP) return;
      // Steeper than 30°, or to the right: not ours.
      if (dx >= 0 || Math.abs(dy) > Math.abs(dx) * TAN30) { start = null; return; }
      swiping = true;
    }
    show(Math.max(dx, -MAX));
  };

  const up = () => {
    if (!start && !swiping) return;
    const fire = swiping && armed;
    reset();
    if (fire) opts.onreply?.();
  };

  node.addEventListener('pointerdown', down);
  node.addEventListener('pointermove', move);
  node.addEventListener('pointerup', up);
  node.addEventListener('pointercancel', reset);

  return {
    update(next: SwipeReplyOptions) { opts = next; },
    destroy() {
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('pointermove', move);
      node.removeEventListener('pointerup', up);
      node.removeEventListener('pointercancel', reset);
      arrow?.remove();
    },
  };
}

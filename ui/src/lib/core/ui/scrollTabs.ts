// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A row of tabs that scrolls sideways when it does not fit (`overflow-x: auto` is the caller's):
// - a clipped edge is marked so the caller fades it: `fade-end` while there is more to the right,
//   `fade-start` (with `fadeStart`) while there is more to the left — the caller's CSS draws the
//   fade (`.tabs:global(.fade-end) { mask-image: … }`);
// - the chosen tab (`[aria-selected="true"]`) is kept in view when `selected` changes: the row
//   alone is moved (`scrollTo`, never `scrollIntoView`, which would move the page around it), as
//   little as it takes;
// - with `wheel`, a mouse's wheel (up and down) moves the row sideways while there is room;
// - the marks follow the row's room: its own size (a ResizeObserver) and `watch`, anything that
//   may widen the tabs (their list, their counts).

import { tick } from 'svelte';
import type { ActionReturn } from 'svelte/action';

export interface ScrollTabsOptions {
  /** The chosen tab: when it changes, it is brought into view. */
  selected?: unknown;
  /** What may change the width of the tabs (their list, their counts): the edges are measured again. */
  watch?: unknown;
  /** A mouse's wheel moves the row sideways. */
  wheel?: boolean;
  /** The start edge is marked too, not only the end. */
  fadeStart?: boolean;
}

/** How far from an edge still counts as at it, in pixels. */
const SLACK = 2;

export function scrollTabs(node: HTMLElement, options: ScrollTabsOptions = {}): ActionReturn<ScrollTabsOptions> {
  let opts = options;

  function edges() {
    const atStart = node.scrollLeft <= SLACK;
    const atEnd = node.scrollLeft + node.clientWidth >= node.scrollWidth - SLACK;
    node.classList.toggle('fade-start', !!opts.fadeStart && !atStart);
    node.classList.toggle('fade-end', !atEnd);
  }

  function chosenIntoView() {
    const on = node.querySelector<HTMLElement>('[aria-selected="true"]');
    if (!on) return;
    const box = node.getBoundingClientRect();
    const r = on.getBoundingClientRect();
    const start = r.left - box.left + node.scrollLeft;
    const end = start + r.width;
    const left = start < node.scrollLeft ? start : end > node.scrollLeft + node.clientWidth ? end - node.clientWidth : null;
    if (left !== null) node.scrollTo?.({ left });
  }

  function onWheel(e: WheelEvent) {
    if (!opts.wheel) return;
    if (node.scrollWidth <= node.clientWidth || Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
    const before = node.scrollLeft;
    node.scrollLeft += e.deltaY;
    if (node.scrollLeft !== before) e.preventDefault();
  }

  node.addEventListener('scroll', edges);
  node.addEventListener('wheel', onWheel, { passive: false });
  const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(edges);
  ro?.observe(node);
  chosenIntoView();
  void tick().then(edges);

  return {
    update(next) {
      const moved = next.selected !== opts.selected;
      opts = next;
      if (moved) chosenIntoView();
      void tick().then(edges);
    },
    destroy() {
      node.removeEventListener('scroll', edges);
      node.removeEventListener('wheel', onWheel);
      ro?.disconnect();
    },
  };
}

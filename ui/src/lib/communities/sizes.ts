// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// One ResizeObserver for every card of the feeds (a switch of sections,
// 2026-10-23): an observer of its own a card had its callback called
// apart, the page's state written after each, so the next card read a
// layout made dirty again — one layout a card, the switch froze. Here the
// cards whose boxes changed are measured in one pass: all of them read,
// then what they write is drawn once.

type Measure = () => void;

const watchers = new Map<Element, Set<Measure>>();
let observer: ResizeObserver | null = null;

function notify(entries: ResizeObserverEntry[]) {
  const due = new Set<Measure>();
  for (const e of entries) for (const fn of watchers.get(e.target) ?? []) due.add(fn);
  for (const fn of due) fn();
}

/** `measure` again whenever one of `elements` changes its size; the returned function stops it. */
export function watchSize(elements: readonly Element[], measure: Measure): () => void {
  if (typeof ResizeObserver === 'undefined' || !elements.length) return () => {};
  observer ??= new ResizeObserver(notify);
  for (const el of elements) {
    let fns = watchers.get(el);
    if (!fns) {
      fns = new Set();
      watchers.set(el, fns);
      observer.observe(el);
    }
    fns.add(measure);
  }
  return () => {
    for (const el of elements) {
      const fns = watchers.get(el);
      if (!fns) continue;
      fns.delete(measure);
      if (!fns.size) {
        watchers.delete(el);
        observer?.unobserve(el);
      }
    }
  };
}

/** Not drawn (in a list kept hidden, `display: none`): nothing to measure, what was measured stays. */
export const undrawn = (el: Element): boolean => el.getClientRects().length === 0;

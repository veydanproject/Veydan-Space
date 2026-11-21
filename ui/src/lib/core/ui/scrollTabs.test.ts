// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import { scrollTabs } from './scrollTabs';

/** A row of tabs as scrollTabs reads it: its scroll and room, its classes. */
function row(scrollLeft: number, clientWidth: number, scrollWidth: number) {
  const classes = new Set<string>();
  const node = {
    scrollLeft, clientWidth, scrollWidth, classes,
    classList: { toggle: (c: string, on: boolean) => (on ? classes.add(c) : classes.delete(c)) },
    addEventListener: () => {},
    removeEventListener: () => {},
    querySelector: (): unknown => null,
    getBoundingClientRect: () => ({ left: 0 }),
    scrollTo: vi.fn((o: { left: number }) => (node.scrollLeft = o.left)),
  };
  return node;
}

describe('scrollTabs', () => {
  it('tells which sides have more of the row, as the fades', async () => {
    const cases: [number, boolean, boolean][] = [[0, false, true], [150, true, true], [300, true, false]];
    for (const [left, before, after] of cases) {
      const node = row(left, 200, 500);
      const onedges = vi.fn();
      const action = scrollTabs(node as unknown as HTMLElement, { fadeStart: true, onedges });
      await tick();
      expect(onedges).toHaveBeenLastCalledWith(before, after);
      expect(node.classes.has('fade-start')).toBe(before);
      expect(node.classes.has('fade-end')).toBe(after);
      action.destroy?.();
    }
  });

  it('tells nothing hidden when the row fits', async () => {
    const node = row(0, 500, 500);
    const onedges = vi.fn();
    scrollTabs(node as unknown as HTMLElement, { onedges });
    await tick();
    expect(onedges).toHaveBeenLastCalledWith(false, false);
  });

  it('brings the chosen tab clear of an arrow at a clipped edge, not past the row', () => {
    // The row shows 300–500 of 1000; the chosen tab is 40 wide at `start`.
    const cases: [number, number | null][] = [
      [480, 392], // past the end: its end stands 72 from the right edge
      [250, 178], // before the start: 72 from the left edge
      [380, null], // inside the clear part: nothing moves
      [960, 800], // the last tab: the row ends, no arrow there
      [20, 0], // the first one: the row starts
    ];
    for (const [start, want] of cases) {
      const node = row(300, 200, 1000);
      const action = scrollTabs(node as unknown as HTMLElement, { selected: 'a', inset: 72 });
      node.querySelector = () => ({ getBoundingClientRect: () => ({ left: start - node.scrollLeft, width: 40 }) });
      action.update?.({ selected: 'b', inset: 72 });
      if (want === null) expect(node.scrollTo).not.toHaveBeenCalled();
      else expect(node.scrollTo).toHaveBeenCalledWith({ left: want });
      action.destroy?.();
    }
  });
});

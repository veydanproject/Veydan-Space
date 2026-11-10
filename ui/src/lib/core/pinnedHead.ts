// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * A table's pinned head has no background of its own: the table's glass is over the window's glow, which no opaque
 * colour repeats. So what scrolls under it is cut at its edge instead, by `clip-path` on the cells: a row under the
 * head (`.pinned-head`, sticky at the top) is cut at the head's bottom edge, and with `column` a column scrolled
 * sideways under the pinned first column is cut at its right edge. A cell clear of both has no clip.
 *
 * `use:pinnedHead` goes on the element that scrolls, with the table inside. Rows and columns that come and go, and
 * sizes that change, are followed by the helper itself.
 */
export interface PinnedHeadOptions {
  /** The first column is pinned too (sticky at the left): the others are cut at its right edge. */
  column?: boolean;
  /** Nothing is cut (the phone's table, with nothing pinned). */
  disabled?: boolean;
}

/** Wide insets on the sides not cut: the cell's line and anything it shows past its box stay. */
const inset = (top: number, left: number) =>
  top || left ? `inset(${top ? `${top}px` : '-100vh'} -100vw -100vh ${left ? `${left}px` : '-100vw'})` : '';

function set(cell: HTMLTableCellElement, clip: string) {
  if (cell.style.clipPath !== clip) cell.style.clipPath = clip;
}

export function pinnedHead(wrap: HTMLElement, options: PinnedHeadOptions = {}) {
  let opts = options;

  function clip() {
    const table = wrap.querySelector('table');
    if (!table) return;
    const head = table.tHead;
    const rows = Array.from(table.tBodies).flatMap((b) => Array.from(b.rows));
    if (opts.disabled) {
      for (const row of [...(head?.rows ?? []), ...rows]) for (const cell of row.cells) set(cell, '');
      return;
    }
    // Read everything first, then write: a write between reads would make each read lay the table out again.
    const ths = head?.rows[0]?.cells;
    const cols: number[] = [];
    if (opts.column && ths && ths.length > 1) {
      const edge = wrap.scrollLeft + ths[0].offsetWidth;
      for (let i = 1; i < ths.length; i++) cols[i] = Math.max(0, Math.min(edge - ths[i].offsetLeft, ths[i].offsetWidth));
    }
    // The head's cells are what is pinned (sticky), not the head itself, which scrolls away with the table: the edge
    // is their bottom. A head that is not drawn (a narrow window folds the rows and hides it) cuts nothing.
    const pinned = head && head.offsetHeight ? Array.from(head.rows, (row) => row.cells[0]).filter(Boolean) : [];
    const bottom = pinned.reduce((b, cell) => Math.max(b, cell.getBoundingClientRect().bottom), -Infinity);
    const tops = rows.map((row) => {
      const r = row.getBoundingClientRect();
      return Math.max(0, Math.min(bottom - r.top, r.height));
    });
    for (const row of head?.rows ?? []) for (let i = 0; i < row.cells.length; i++) set(row.cells[i], inset(0, cols[i] ?? 0));
    rows.forEach((row, n) => {
      for (let i = 0; i < row.cells.length; i++) set(row.cells[i], inset(tops[n], cols[i] ?? 0));
    });
  }

  const ro = new ResizeObserver(clip);
  ro.observe(wrap);
  // Rows and columns drawn anew: the table is watched for them, and a new table is observed for its size.
  let observed: HTMLTableElement | null = null;
  function watchTable() {
    const table = wrap.querySelector('table');
    if (table === observed) return;
    if (observed) ro.unobserve(observed);
    observed = table;
    if (table) ro.observe(table);
  }
  const mo = new MutationObserver(() => { watchTable(); clip(); });
  mo.observe(wrap, { childList: true, subtree: true });
  wrap.addEventListener('scroll', clip, { passive: true });
  watchTable();
  clip();

  return {
    update(next: PinnedHeadOptions = {}) {
      opts = next;
      clip();
    },
    destroy() {
      ro.disconnect();
      mo.disconnect();
      wrap.removeEventListener('scroll', clip);
    },
  };
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// How pictures of one album are laid out: rows of one to three, the row
// with fewer pictures first, so the first picture gets the most room.
//
//   1 → [1]        4 → [2, 2]        7 → [1, 3, 3]      10 → [1, 3, 3, 3]
//   2 → [2]        5 → [2, 3]        8 → [2, 3, 3]
//   3 → [1, 2]     6 → [3, 3]        9 → [3, 3, 3]

export const MAX_PER_ROW = 3;

/** How many pictures stand in each row, top to bottom. */
export function mosaicRows(count: number): number[] {
  if (count <= 0) return [];
  if (count <= 2) return [count];
  if (count === 3) return [1, 2];
  if (count === 4) return [2, 2];
  const full = Math.floor(count / MAX_PER_ROW);
  const rest = count % MAX_PER_ROW;
  return rest ? [rest, ...Array(full).fill(MAX_PER_ROW)] : Array(full).fill(MAX_PER_ROW);
}

/** Row by row, the indices of the pictures in it. */
export function mosaic<T>(items: T[]): T[][] {
  const out: T[][] = [];
  let at = 0;
  for (const n of mosaicRows(items.length)) {
    out.push(items.slice(at, at + n));
    at += n;
  }
  return out;
}

/** Height of a row in pixels: the fewer pictures, the taller. */
export function rowHeight(inRow: number, rows: number): number {
  if (rows === 1 && inRow === 1) return 0; // one picture alone keeps its own proportions
  return inRow === 1 ? 220 : inRow === 2 ? 160 : 120;
}

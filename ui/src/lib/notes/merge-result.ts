// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Assembles the final text from 3-way merge blocks and per-block choices.
// Shared by the desktop merge panel and the mobile conflict sheet.

import type { MergeBlock } from '$lib/notes/types';

export type MergeSide = 'ours' | 'theirs';
export type MergeChoice = MergeSide | 'both';
/** One entry per block; normal blocks stay null. */
export type MergeSelections = (MergeChoice | null)[];

export function emptySelections(blocks: MergeBlock[]): MergeSelections {
  return blocks.map(() => null);
}

/** Picks `side` for every conflict block. */
export function selectAll(blocks: MergeBlock[], side: MergeSide): MergeSelections {
  return selectAllChoice(blocks, side);
}

/** Picks one whole-block choice, including keeping both sides. */
export function selectAllChoice(blocks: MergeBlock[], choice: MergeChoice): MergeSelections {
  return blocks.map((b) => (b.kind === 'conflict' ? choice : null));
}

/** Which lines of a conflict are included. Order is fixed: ours, then theirs. */
export type LinePick = { ours: boolean[]; theirs: boolean[] };

/** Lines of a block side. A trailing newline is not an extra empty line. */
export function textLines(text: string): string[] {
  if (text === '') return [];
  const parts = text.split('\n');
  if (text.endsWith('\n')) parts.pop();
  return parts;
}

/** Line flags that match a whole-side choice. */
export function linePickFor(ours: string, theirs: string, choice: MergeChoice): LinePick {
  return {
    ours: textLines(ours).map(() => choice === 'ours' || choice === 'both'),
    theirs: textLines(theirs).map(() => choice === 'theirs' || choice === 'both'),
  };
}

/** Joins included lines. Empty selection yields an empty string. */
export function joinPickedLines(ours: string, theirs: string, pick: LinePick): string {
  const lines = [
    ...textLines(ours).filter((_, i) => pick.ours[i]),
    ...textLines(theirs).filter((_, i) => pick.theirs[i]),
  ];
  if (lines.length === 0) return '';
  const nl = ours.endsWith('\n') || theirs.endsWith('\n');
  return nl ? `${lines.join('\n')}\n` : lines.join('\n');
}

/** Flags drawn on the lines: a manual pick, a whole-side choice, or nothing yet. */
export function shownLinePick(block: MergeBlock, choice: MergeChoice | null, pick: LinePick | null): LinePick {
  if (block.kind !== 'conflict') return { ours: [], theirs: [] };
  if (pick) return pick;
  if (!choice) {
    return {
      ours: textLines(block.ours).map(() => false),
      theirs: textLines(block.theirs).map(() => false),
    };
  }
  return linePickFor(block.ours, block.theirs, choice);
}

function sameFlags(a: boolean[], b: boolean[]): boolean {
  return a.length === b.length && a.every((v, i) => v === b[i]);
}

/** Next choice after toggling one line. A full side collapses back to that choice. */
export function nextLineToggle(
  block: MergeBlock,
  current: LinePick,
  side: 'ours' | 'theirs',
  line: number,
): { choice: MergeChoice | null; pick: LinePick | null } | null {
  if (block.kind !== 'conflict') return null;
  const next: LinePick = { ours: [...current.ours], theirs: [...current.theirs] };
  const list = side === 'ours' ? next.ours : next.theirs;
  if (line < 0 || line >= list.length) return null;
  list[line] = !list[line];
  const whole = (['ours', 'theirs', 'both'] as const).find((choice) => {
    const full = linePickFor(block.ours, block.theirs, choice);
    return sameFlags(full.ours, next.ours) && sameFlags(full.theirs, next.theirs);
  });
  if (whole) return { choice: whole, pick: null };
  return { choice: null, pick: next };
}

export function unresolvedCount(
  blocks: MergeBlock[],
  selections: MergeSelections,
  picks: (LinePick | null)[] = [],
): number {
  return blocks.filter((b, i) => b.kind === 'conflict' && selections[i] == null && picks[i] == null).length;
}

export function isComplete(
  blocks: MergeBlock[],
  selections: MergeSelections,
  picks: (LinePick | null)[] = [],
): boolean {
  return unresolvedCount(blocks, selections, picks) === 0;
}

/** Block texts keep their trailing newlines, so plain concatenation rebuilds the file. */
export function buildMergeContent(blocks: MergeBlock[], selections: MergeSelections): string {
  return buildResolvedContent(blocks, selections, []);
}

/** Like `buildMergeContent`, but a line pick on a block replaces the whole-side choice. */
export function buildResolvedContent(
  blocks: MergeBlock[],
  selections: MergeSelections,
  picks: (LinePick | null)[],
): string {
  return blocks
    .map((b, i) => {
      if (b.kind === 'normal') return b.text;
      const pick = picks[i];
      if (pick) return joinPickedLines(b.ours, b.theirs, pick);
      switch (selections[i]) {
        case 'theirs': return b.theirs;
        case 'both': return b.ours + b.theirs;
        default: return b.ours;
      }
    })
    .join('');
}

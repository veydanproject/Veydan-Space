// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { MergeBlock } from '$lib/notes/types';
import { buildMergeContent, emptySelections, isComplete, selectAll, unresolvedCount } from './merge-result';

const normal = (text: string): MergeBlock => ({ kind: 'normal', text, ours: '', theirs: '' });
const conflict = (ours: string, theirs: string): MergeBlock => ({ kind: 'conflict', text: '', ours, theirs });

const blocks: MergeBlock[] = [normal('a\n'), conflict('mine\n', 'theirs\n'), normal('z\n'), conflict('m2\n', 't2\n')];

describe('merge-result', () => {
  it('counts unresolved conflict blocks only', () => {
    expect(unresolvedCount(blocks, emptySelections(blocks))).toBe(2);
    expect(isComplete(blocks, emptySelections(blocks))).toBe(false);
    expect(isComplete([normal('x')], emptySelections([normal('x')]))).toBe(true);
  });

  it('selectAll picks one side for every conflict', () => {
    expect(selectAll(blocks, 'ours')).toEqual([null, 'ours', null, 'ours']);
    expect(selectAll(blocks, 'theirs')).toEqual([null, 'theirs', null, 'theirs']);
    expect(isComplete(blocks, selectAll(blocks, 'theirs'))).toBe(true);
  });

  it('builds content from choices and keeps normal text', () => {
    expect(buildMergeContent(blocks, selectAll(blocks, 'ours'))).toBe('a\nmine\nz\nm2\n');
    expect(buildMergeContent(blocks, selectAll(blocks, 'theirs'))).toBe('a\ntheirs\nz\nt2\n');
    expect(buildMergeContent(blocks, [null, 'both', null, 'theirs'])).toBe('a\nmine\ntheirs\nz\nt2\n');
  });

  it('falls back to ours for an unresolved block', () => {
    expect(buildMergeContent(blocks, emptySelections(blocks))).toBe('a\nmine\nz\nm2\n');
  });
});

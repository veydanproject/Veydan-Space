// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { appendPage, keyQuery, shownReaders } from './readers';
import type { ReaderView } from './api';

const r = (c: string, since = 0): ReaderView => ({ pubkey: c.repeat(64).slice(0, 64), since, levels: [] });

describe('the list of the readers', () => {
  it('asks the core for a start of a key only', () => {
    expect(keyQuery(' AbCd ')).toBe('abcd');
    expect(keyQuery('abc'), 'too short: a name').toBe('');
    expect(keyQuery('Вера')).toBe('');
    expect(keyQuery('alice')).toBe('');
    expect(keyQuery('a'.repeat(65))).toBe('');
  });

  it('adds a page without a reader twice', () => {
    expect(appendPage([r('1'), r('2')], [r('2'), r('3')]).map((x) => x.pubkey[0])).toEqual(['1', '2', '3']);
  });

  it('shows the core’s hits first, then the loaded ones that match, each once', () => {
    const rows = [r('1'), r('2'), r('3')];
    expect(shownReaders(rows, [], '  ', () => false)).toEqual(rows);
    const matches = (k: string) => k.startsWith('2') || k.startsWith('3');
    expect(shownReaders(rows, [r('3'), r('9')], 'x', matches).map((x) => x.pubkey[0])).toEqual(['3', '9', '2']);
    expect(shownReaders(rows, [], 'x', () => false)).toEqual([]);
  });
});

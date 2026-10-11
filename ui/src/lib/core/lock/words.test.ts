// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { looksLikeCode, looksLikeWords, pickPositions, splitWords, wordsMatch } from './words';

const WORDS = 'abandon math mimic master filter design carbon crystal rookie group knife young';

describe('the recovery key of 12 words', () => {
  it('is told from a code by its shape', () => {
    expect(looksLikeWords(WORDS)).toBe(true);
    expect(looksLikeWords(`  Abandon MATH mimic\nmaster filter design carbon crystal rookie group knife young `)).toBe(true);
    expect(looksLikeWords('abandon math mimic')).toBe(false);
    expect(looksLikeWords('ABCD-EFGH-JKMN-PQRS-TVWX-YZ00')).toBe(false);
    expect(looksLikeCode('ABCD-EFGH-JKMN-PQRS-TVWX-YZ00')).toBe(true);
    expect(looksLikeCode('abcd efgh jkmn pqrs tvwx yz00')).toBe(true);
    expect(looksLikeCode(WORDS)).toBe(false);
    expect(splitWords(WORDS)).toHaveLength(12);
  });

  it('asks three distinct words back, in order, and checks them without case', () => {
    let n = 0;
    const seq = [0.9, 0.1, 0.5, 0.3, 0.7, 0.2, 0.8, 0.4, 0.6, 0.05, 0.95, 0.5];
    const positions = pickPositions(12, () => seq[n++ % seq.length]);
    expect(positions).toHaveLength(3);
    expect(new Set(positions).size).toBe(3);
    expect([...positions].sort((a, b) => a - b)).toEqual(positions);
    expect(positions.every((p) => p >= 1 && p <= 12)).toBe(true);
    const words = splitWords(WORDS);
    const typed = positions.map((p) => ` ${words[p - 1].toUpperCase()} `);
    expect(wordsMatch(words, positions, typed)).toBe(true);
    typed[1] = 'young';
    expect(wordsMatch(words, positions, typed)).toBe(positions[1] === 12);
    expect(wordsMatch(words, positions, ['', '', ''])).toBe(false);
    expect(pickPositions(2)).toHaveLength(2);
  });
});

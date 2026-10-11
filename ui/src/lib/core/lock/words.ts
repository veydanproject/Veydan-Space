// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The Recovery Key of 12 words on the screens of the lock (platform-spec 8.1).

/** How many words a Recovery Key has. */
export const RECOVERY_WORDS = 12;

/** The words of a typed key: any whitespace, any case, nothing else changed. */
export function splitWords(text: string): string[] {
  return text.trim().split(/\s+/).filter(Boolean);
}

/** Whether `text` has the shape of a Recovery Key: 12 words of letters. */
export function looksLikeWords(text: string): boolean {
  const words = splitWords(text);
  return words.length === RECOVERY_WORDS && words.every((w) => /^[a-z]+$/i.test(w));
}

/** Whether `text` has the shape of a recovery code of the older lock: 24 characters in groups. */
export function looksLikeCode(text: string): boolean {
  return text.replace(/[\s-]/g, '').length === 24;
}

/**
 * Three distinct positions (1-based) of `count` words, in order: what the
 * check after «Я записал(а)» asks to type back.
 */
export function pickPositions(count: number, random: () => number = Math.random): number[] {
  const all = Array.from({ length: count }, (_, i) => i + 1);
  for (let i = all.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1));
    [all[i], all[j]] = [all[j], all[i]];
  }
  return all.slice(0, Math.min(3, count)).sort((a, b) => a - b);
}

/** Whether the words typed at `positions` are those of `words`. */
export function wordsMatch(words: string[], positions: number[], typed: string[]): boolean {
  return positions.every((p, i) => (typed[i] ?? '').trim().toLowerCase() === words[p - 1]);
}

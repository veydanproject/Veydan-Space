// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The strip of reactions at the top of a message's menu: the emoji I use
// most, then the usual ones, so a new device has six from the start.

export const QUICK_DEFAULTS = ['❤️', '👍', '😂', '😮', '😢', '🔥'] as const;
export const QUICK_SIZE = 6;

/** Six emoji: the most popular first, filled up with the defaults in order, none twice. */
export function quickSet(popular: string[]): string[] {
  const out: string[] = [];
  for (const e of [...popular, ...QUICK_DEFAULTS]) {
    if (out.length === QUICK_SIZE) break;
    if (e && !out.includes(e)) out.push(e);
  }
  return out;
}

/**
 * The message a reaction from the menu of `pressed` lands on. An album's
 * parts are messages of their own, but its reactions are kept on its last
 * part, the one its bubble shows them under: so whichever part is pressed,
 * the reaction goes there.
 */
export function reactionTarget<M>(pressed: M, album?: readonly M[]): M {
  return album?.length ? album[album.length - 1] : pressed;
}

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

/**
 * The part of an album a press on its bubble belongs to: the one whose
 * tile or caption carries `id`. Anything else in the bubble (its padding,
 * the author, the time, the reactions, the gaps between tiles) gives
 * `fallback`.
 */
export function pressedPart<M extends { id: string }>(parts: readonly M[], id: string | null | undefined, fallback: M): M {
  return (id && parts.find((m) => m.id === id)) || fallback;
}

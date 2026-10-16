// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { toast } from '$lib/core/ui/toast.svelte';
import type { MessengerMessage } from '../api';

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

/** Three different reactions of mine on a message at most (design kit): a fourth new emoji is refused, and the toast says why. */
export const MAX_OWN_REACTIONS = 3;
export function reactionAllowed(m: MessengerMessage, emoji: string): boolean {
  const mine = m.reactions.filter((r) => r.mine);
  if (mine.some((r) => r.emoji === emoji) || mine.length < MAX_OWN_REACTIONS) return true;
  toast.info((get(t) as (k: string) => string)('msg_react_limit'));
  return false;
}

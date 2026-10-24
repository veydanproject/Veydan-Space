// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The people of one level, in its dialog (communities-wire §14.1,
// `set_levels`): who holds it by the journal, whom a moderator may give it
// to, and the levels a person is left with after. A level besides the base
// one is held only by a journal entry, so the list is whole; the base level
// is everybody's and has no list.

import type { ChannelDetail } from '../api';
import { BASE_LEVEL, levelsOfPerson } from './sections';
import type { StaffViewLv } from './types';

/** A hex key typed whole; a level is given by it. */
export const isHexKey = (s: string): boolean => /^[0-9a-f]{64}$/i.test(s.trim());

/** Who holds the level by the journal. */
export function holders(d: ChannelDetail, level: number): string[] {
  if (level === BASE_LEVEL) return [];
  return (d.staff as StaffViewLv[]).filter((s) => (s.levels ?? []).includes(level)).map((s) => s.pubkey);
}

/** The levels a person has with this one given (`true`) or taken (`false`); the base one always stays. */
export function levelsWith(d: ChannelDetail, who: string, level: number, give: boolean): number[] {
  const now = levelsOfPerson(d, who).filter((id) => id !== level || id === BASE_LEVEL);
  return [...new Set(give ? [...now, level] : now)].sort((a, b) => a - b);
}

/**
 * Whom the level may be given to, of those the page knows (the channel's
 * journal, then my contacts): not the holders, not the banned, only
 * those `allowed` says (a moderator gives levels to the roles below the
 * team), each once, in that order.
 */
export function candidates(known: readonly string[], held: readonly string[], banned: ReadonlySet<string>, allowed: (who: string) => boolean): string[] {
  const out = new Set<string>();
  const skip = new Set(held);
  for (const k of known) if (!skip.has(k) && !banned.has(k) && allowed(k)) out.add(k);
  return [...out];
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The sections of a channel in its feed (communities-wire §14.3, §14.7, spec
// §12): the tiles of the button in the head, which posts each shows, the
// mark of a post's section, and where a post goes. A reader's feed is the
// union of the sections they are in; a section they lack is a locked tile,
// and a hidden one they lack is not given at all.

import type { ChannelDetail, FeedItemView } from '../api';
import { ordered } from './look';
import type { ChannelDetailLv, DetailLevels, FeedItemViewLv, LevelView, ScopeView } from './types';

/** The base section, of everyone who entered. */
export const EVERYONE = 'everyone';
/** The base level, of everyone who entered: it always gives `everyone`. */
export const BASE_LEVEL = 0;

/** The levels and sections of a channel; empty for a core of before them. */
export function levelsOf(d: ChannelDetail | null | undefined): DetailLevels {
  const x = (d ?? {}) as Partial<ChannelDetailLv>;
  return { scopes: x.scopes ?? [], levels: x.levels ?? [], my_levels: x.my_levels ?? [] };
}

/** The section of a post or a comment; one without is of everyone. */
export const scopeOf = (item: Pick<FeedItemView, 'id'>): string => (item as FeedItemViewLv).scope_id || EVERYONE;

/** The button of the sections shows only when the channel has a section besides everyone's. */
export const hasSections = (d: ChannelDetail | null | undefined): boolean => levelsOf(d).scopes.length > 1;

export type SectionTile =
  | { kind: 'all' }
  | { kind: 'open'; scope: ScopeView }
  | { kind: 'locked'; scope: ScopeView; levels: LevelView[] };

/** The levels that give a section, the base one last (it gives everyone's alone). */
export function levelsGranting(levels: readonly LevelView[], scopeId: string): LevelView[] {
  return levels.filter((l) => l.scopes.includes(scopeId) || (l.id === BASE_LEVEL && scopeId === EVERYONE));
}

/**
 * «Все», then the sections in the owner's order: one I am in opens its
 * feed, one I lack is locked, with the levels that would give it. A hidden
 * section is given to its members alone (the core leaves it out for the
 * rest); one that came anyway is not shown.
 */
export function sectionTiles(scopes: readonly ScopeView[], levels: readonly LevelView[]): SectionTile[] {
  if (scopes.length <= 1) return [];
  return [
    { kind: 'all' },
    ...ordered(scopes).filter((s) => s.mine || !s.hidden || s.scope_id === EVERYONE).map((scope): SectionTile =>
      scope.mine || scope.scope_id === EVERYONE
        ? { kind: 'open', scope }
        : { kind: 'locked', scope, levels: levelsGranting(levels, scope.scope_id).filter((l) => l.id !== BASE_LEVEL) }),
  ];
}

/** The posts a section shows: all (`null`) or one section's. */
export function inScope<T extends Pick<FeedItemView, 'id'>>(items: readonly T[], scope: string | null): T[] {
  return scope === null ? [...items] : items.filter((x) => scopeOf(x) === scope);
}

/** The section of a post, to mark it with in «Все»; `null` for one unknown here. */
export function sectionOf(scopes: readonly ScopeView[], item: Pick<FeedItemView, 'id'>): ScopeView | null {
  const id = scopeOf(item);
  return scopes.find((s) => s.scope_id === id) ?? null;
}

/** The title of a post's section to mark it with; `null` for everyone's or one unknown. */
export function sectionTitle(scopes: readonly ScopeView[], item: Pick<FeedItemView, 'id'>): string | null {
  const id = scopeOf(item);
  if (id === EVERYONE) return null;
  return scopes.find((s) => s.scope_id === id)?.title ?? null;
}

/** May I post into this section: the core says it (`post`, 2026-10-23); a core of before it, by membership. */
export const postable = (s: ScopeView): boolean => s.post ?? (s.mine || s.scope_id === EVERYONE);

/**
 * Where a new post goes (the owner's word, 2026-10-23): into the section
 * shown; `null` when I may not post there (the page then shows no
 * composer), and in «Все» (`null`) of a channel with sections, which asks
 * to open a section first (the owner's word, later the same day). A
 * channel without sections (everyone's alone, or a core of before them)
 * posts to everyone's.
 */
export function postTarget(scopes: readonly ScopeView[], shown: string | null): string | null {
  if (!scopes.length) return EVERYONE;
  if (shown === null && scopes.length > 1) return null;
  const id = shown ?? EVERYONE;
  const s = scopes.find((x) => x.scope_id === id);
  return s && postable(s) ? id : null;
}

/** The levels of a person as the journal says them (a staff entry's `levels`); the base level is always one. */
export function levelsOfPerson(d: ChannelDetail, pubkey: string): number[] {
  const s = d.staff.find((x) => x.pubkey === pubkey) as (ChannelDetail['staff'][number] & { levels?: number[] }) | undefined;
  return [...new Set([BASE_LEVEL, ...(s?.levels ?? [])])].sort((a, b) => a - b);
}

/** The names of the levels a person has besides the base one, for the chips of their row. */
export function levelNames(levels: readonly LevelView[], ids: readonly number[]): string[] {
  return ids.filter((id) => id !== BASE_LEVEL).map((id) => levels.find((l) => l.id === id)?.name).filter((n): n is string => !!n);
}

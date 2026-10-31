// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A reader asks for a level (communities-wire §14.4, `channel.level_request`):
// which levels are worth asking for (those that give more than mine do),
// what each would add, and the request sent, kept on this device until the
// channel tells of a change (`channel.updated`: a moderator gave it, or
// anything else; a refusal comes as a note of its own).

import { BASE_LEVEL, EVERYONE } from './sections';
import { RIGHTS } from './editor';
import type { LevelRights, LevelView, ScopeView } from './types';

/** The sections and rights a set of levels gives; the base level always among them. */
export function grants(levels: readonly LevelView[], ids: readonly number[]): { scopes: Set<string>; can: LevelRights } {
  const mine = levels.filter((l) => l.id === BASE_LEVEL || ids.includes(l.id));
  const scopes = new Set<string>([EVERYONE, ...mine.flatMap((l) => l.scopes)]);
  const can: LevelRights = { comment: false, discuss: false, post: false };
  for (const l of mine) for (const r of RIGHTS) can[r] ||= l.can[r];
  return { scopes, can };
}

/** What a level would add to mine: the sections it opens that I lack, the rights I lack. */
export function adds(level: LevelView, levels: readonly LevelView[], myLevels: readonly number[], scopes: readonly ScopeView[]): { scopes: ScopeView[]; rights: (keyof LevelRights)[] } {
  const have = grants(levels, myLevels);
  return {
    scopes: level.scopes.filter((s) => !have.scopes.has(s)).map((id) => scopes.find((s) => s.scope_id === id)).filter((s): s is ScopeView => !!s),
    rights: RIGHTS.filter((r) => level.can[r] && !have.can[r]),
  };
}

/** The levels worth asking for: not the base one, not one I have, and one that gives me something more. */
export function requestable(levels: readonly LevelView[], myLevels: readonly number[], scopes: readonly ScopeView[]): LevelView[] {
  return levels
    .filter((l) => l.id !== BASE_LEVEL && !myLevels.includes(l.id))
    .filter((l) => { const a = adds(l, levels, myLevels, scopes); return a.scopes.length > 0 || a.rights.length > 0; })
    .sort((a, b) => a.id - b.id);
}

/** The level to offer first for a locked section: the first worth asking for that opens it. */
export function levelFor(scopeId: string, levels: readonly LevelView[], myLevels: readonly number[], scopes: readonly ScopeView[]): LevelView | null {
  return requestable(levels, myLevels, scopes).find((l) => l.scopes.includes(scopeId)) ?? null;
}

/** Requests sent from this device, by channel: kept until the channel tells of a change. */
export type Asked = Record<string, number[]>;

export const markAsked = (asked: Asked, communityId: string, level: number): Asked =>
  ({ ...asked, [communityId]: [...new Set([...(asked[communityId] ?? []), level])] });

export function clearAsked(asked: Asked, communityId: string): Asked {
  if (!(communityId in asked)) return asked;
  const { [communityId]: _gone, ...rest } = asked;
  return rest;
}

/**
 * The levels I wait for an answer on: asked from this device since the
 * channel last changed, or waiting by the channel's own word (its
 * `my_level_requests`, which a restart keeps).
 */
export function waitingLevels(asked: Asked, communityId: string, mine: readonly { level: number; status: string }[] | null | undefined): number[] {
  const kept = (mine ?? []).filter((r) => r.status === 'pending').map((r) => r.level);
  return [...new Set([...(asked[communityId] ?? []), ...kept])];
}

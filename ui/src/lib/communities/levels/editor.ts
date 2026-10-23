// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The owner's editor of levels and sections (communities-wire §14.1): what
// a form may send, said before the core refuses it. The core's limits:
// a level's name up to 40 characters, up to 64 levels (ids 0…63), up to 16
// sections, a section's title up to 80; the base level (id 0) always gives
// the section of everyone and is never deleted.

import { BASE_LEVEL, EVERYONE } from './sections';
import type { LevelInput, LevelRights, LevelView, ScopeView } from './types';

export const LEVEL_NAME_MAX = 40;
export const SCOPE_TITLE_MAX = 80;
export const MAX_LEVELS = 64;
export const MAX_SCOPES = 16;

/** What is wrong with a level a form would send; `null`: nothing. */
export type LevelProblem = 'name_empty' | 'name_long' | 'name_taken' | 'level_limit';
/** What is wrong with a new section; `null`: nothing. */
export type ScopeProblem = 'title_empty' | 'title_long' | 'scope_limit';

/** The rights a level gives, in the order the editor lists them. */
export const RIGHTS: (keyof LevelRights)[] = ['comment', 'discuss', 'post'];

/** A level as it goes to the core: its name trimmed, its sections once each, the base one with everyone's first. */
export function normalizeLevel(input: LevelInput): LevelInput {
  const scopes = [...new Set(input.scopes)].filter((s) => s !== EVERYONE);
  const out: LevelInput = { name: input.name.trim(), scopes: input.id === BASE_LEVEL ? [EVERYONE, ...scopes] : scopes, can: { ...input.can } };
  if (input.id !== undefined) out.id = input.id;
  return out;
}

/** The length a person sees: a name of emoji counts each as one. */
const length = (s: string) => [...s].length;

export function checkLevel(input: LevelInput, levels: readonly LevelView[]): LevelProblem | null {
  const name = input.name.trim();
  if (!name) return 'name_empty';
  if (length(name) > LEVEL_NAME_MAX) return 'name_long';
  const same = name.toLocaleLowerCase();
  if (levels.some((l) => l.id !== input.id && l.name.trim().toLocaleLowerCase() === same)) return 'name_taken';
  if (input.id === undefined && levels.length >= MAX_LEVELS) return 'level_limit';
  return null;
}

export function checkScope(title: string, scopes: readonly ScopeView[]): ScopeProblem | null {
  const t = title.trim();
  if (!t) return 'title_empty';
  if (length(t) > SCOPE_TITLE_MAX) return 'title_long';
  if (scopes.length >= MAX_SCOPES) return 'scope_limit';
  return null;
}

/** How many levels give a section: what deleting it takes from people. */
export const grantingCount = (levels: readonly LevelView[], scopeId: string): number =>
  levels.filter((l) => l.id !== BASE_LEVEL && l.scopes.includes(scopeId)).length;

/** The levels in the order the editor shows them: the base one first, then by id. */
export const orderedLevels = (levels: readonly LevelView[]): LevelView[] =>
  [...levels].sort((a, b) => Number(b.id === BASE_LEVEL) - Number(a.id === BASE_LEVEL) || a.id - b.id);

/** A new level's form: no sections, the right to comment. */
export const blankLevel = (): LevelInput => ({ name: '', scopes: [], can: { comment: true, discuss: false, post: false } });

/** A level's form from the level. */
export const formOf = (l: LevelView): LevelInput => ({ id: l.id, name: l.name, scopes: l.scopes.filter((s) => s !== EVERYONE), can: { ...l.can } });

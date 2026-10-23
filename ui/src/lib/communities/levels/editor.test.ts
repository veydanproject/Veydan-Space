// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The editor of levels and sections says what the core would refuse
// before it is sent (communities-wire §14.1).

import { describe, expect, it } from 'vitest';
import { EVERYONE } from './sections';
import { MAX_LEVELS, MAX_SCOPES, blankLevel, checkLevel, checkScope, formOf, grantingCount, normalizeLevel, orderedLevels } from './editor';
import type { LevelView, ScopeView } from './types';

const level = (id: number, name: string, scopes: string[] = []): LevelView => ({ id, name, scopes, can: { comment: true, discuss: false, post: false }, members: 0 });
const levels = [level(2, 'Меценаты', ['premium', 'club']), level(0, 'Читатели', [EVERYONE]), level(1, 'Поддержка', ['premium'])];
const scope = (scope_id: string): ScopeView => ({ scope_id, title: scope_id, scheme: 'sd-v1', history: 'open', mine: true });

describe('a level in the editor', () => {
  it('needs a name, of up to 40 characters, emoji counted as one', () => {
    expect(checkLevel({ ...blankLevel(), name: '   ' }, levels)).toBe('name_empty');
    expect(checkLevel({ ...blankLevel(), name: 'x'.repeat(41) }, levels)).toBe('name_long');
    expect(checkLevel({ ...blankLevel(), name: '⭐'.repeat(40) }, levels)).toBeNull();
    expect(checkLevel({ ...blankLevel(), name: '  Ученики ' }, levels)).toBeNull();
  });

  it('does not repeat another level’s name, but keeps its own', () => {
    expect(checkLevel({ ...blankLevel(), name: 'поддержка' }, levels)).toBe('name_taken');
    expect(checkLevel({ ...formOf(levels[2]), name: 'Поддержка' }, levels)).toBeNull();
  });

  it('is not made past 64 levels; one changed still is', () => {
    const full = Array.from({ length: MAX_LEVELS }, (_, i) => level(i, `L${i}`));
    expect(checkLevel({ ...blankLevel(), name: 'Новый' }, full)).toBe('level_limit');
    expect(checkLevel({ ...formOf(full[5]), name: 'Другой' }, full)).toBeNull();
  });

  it('goes trimmed, its sections once each; the base level keeps everyone’s first', () => {
    expect(normalizeLevel({ name: ' A ', scopes: ['p', 'p', EVERYONE], can: { comment: true, discuss: false, post: false } }))
      .toEqual({ name: 'A', scopes: ['p'], can: { comment: true, discuss: false, post: false } });
    expect(normalizeLevel({ id: 0, name: 'Все', scopes: ['p'], can: { comment: true, discuss: false, post: false } }).scopes).toEqual([EVERYONE, 'p']);
    expect(formOf(levels[1]).scopes).toEqual([]);
  });

  it('is listed with the base level first, then by id', () => {
    expect(orderedLevels(levels).map((l) => l.id)).toEqual([0, 1, 2]);
  });
});

describe('a section in the editor', () => {
  it('needs a title of up to 80 characters, and no more than 16 sections', () => {
    expect(checkScope(' ', [])).toBe('title_empty');
    expect(checkScope('x'.repeat(81), [])).toBe('title_long');
    expect(checkScope('Премиум', Array.from({ length: MAX_SCOPES }, (_, i) => scope(`s${i}`)))).toBe('scope_limit');
    expect(checkScope('Премиум', [scope(EVERYONE)])).toBeNull();
  });

  it('counts the levels that open it, for the question before deleting it', () => {
    expect(grantingCount(levels, 'premium')).toBe(2);
    expect(grantingCount(levels, 'club')).toBe(1);
    expect(grantingCount(levels, EVERYONE)).toBe(0);
  });
});

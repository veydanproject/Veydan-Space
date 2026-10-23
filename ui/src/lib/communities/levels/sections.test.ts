// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The tiles of the sections and what each shows (communities-wire §14.3, §14.7).

import { describe, expect, it } from 'vitest';
import type { FeedItemView } from '../api';
import { EVERYONE, inScope, levelsOf, postTarget, sectionOf, sectionTiles, sectionTitle } from './sections';
import type { LevelView, ScopeView } from './types';

const scope = (scope_id: string, title: string, mine: boolean, extra: Partial<ScopeView> = {}): ScopeView => ({ scope_id, title, scheme: 'sd-v1', history: 'open', mine, ...extra });
const level = (id: number, name: string, scopes: string[]): LevelView => ({ id, name, scopes, can: { comment: true, discuss: false, post: false }, members: 0 });
const post = (id: string, scope_id?: string) => ({ id, scope_id }) as unknown as FeedItemView;

const scopes = [scope(EVERYONE, '', true), scope('premium', 'Премиум', true), scope('club', 'Клуб', false), scope('staff', 'Команда', false)];
const levels = [level(0, 'Читатели', [EVERYONE]), level(1, 'Поддержка', ['premium']), level(2, 'Меценаты', ['premium', 'club'])];
const tileKeys = (s: readonly ScopeView[]) => sectionTiles(s, levels).map((c) => (c.kind === 'all' ? 'all' : `${c.kind}:${c.scope.scope_id}`));

describe('the tiles of the sections', () => {
  it('are none while the channel has only everyone’s section', () => {
    expect(sectionTiles([scope(EVERYONE, '', true)], levels)).toEqual([]);
    expect(sectionTiles([], levels)).toEqual([]);
  });

  it('say «all», then each section in its order: mine open, the others locked with the levels that open them', () => {
    expect(tileKeys(scopes)).toEqual(['all', `open:${EVERYONE}`, 'open:premium', 'locked:club', 'locked:staff']);
    const tiles = sectionTiles(scopes, levels);
    const club = tiles.find((c) => c.kind === 'locked' && c.scope.scope_id === 'club');
    expect(club?.kind === 'locked' && club.levels.map((l) => l.name)).toEqual(['Меценаты']);
    const staff = tiles.find((c) => c.kind === 'locked' && c.scope.scope_id === 'staff');
    expect(staff?.kind === 'locked' && staff.levels).toEqual([]);
  });

  it('follow the owner’s order, everyone’s where he put it', () => {
    const placed = [scope(EVERYONE, '', true, { order: 2 }), scope('premium', 'Премиум', true, { order: 0 }), scope('club', 'Клуб', false, { order: 1 })];
    expect(tileKeys(placed)).toEqual(['all', 'open:premium', 'locked:club', `open:${EVERYONE}`]);
  });

  it('show a hidden section to its members only', () => {
    const hidden = [...scopes, scope('mods', 'Модераторская', true, { hidden: true }), scope('secret', '', false, { hidden: true })];
    expect(tileKeys(hidden)).toEqual(['all', `open:${EVERYONE}`, 'open:premium', 'locked:club', 'locked:staff', 'open:mods']);
  });

  it('filter the posts read: all, or one section’s, a post without a section being everyone’s', () => {
    const items = [post('a'), post('b', 'premium'), post('c', EVERYONE), post('d', 'premium')];
    expect(inScope(items, null).map((x) => x.id)).toEqual(['a', 'b', 'c', 'd']);
    expect(inScope(items, 'premium').map((x) => x.id)).toEqual(['b', 'd']);
    expect(inScope(items, EVERYONE).map((x) => x.id)).toEqual(['a', 'c']);
    expect(inScope(items, 'club')).toEqual([]);
  });

  it('mark a post with its section, everyone’s too; the title of one not everyone’s', () => {
    expect(sectionOf(scopes, post('b', 'premium'))?.title).toBe('Премиум');
    expect(sectionOf(scopes, post('a'))?.scope_id).toBe(EVERYONE);
    expect(sectionOf(scopes, post('x', 'gone'))).toBeNull();
    expect(sectionTitle(scopes, post('b', 'premium'))).toBe('Премиум');
    expect(sectionTitle(scopes, post('a'))).toBeNull();
    expect(sectionTitle(scopes, post('x', 'gone'))).toBeNull();
  });

  it('post into the section shown, nowhere from «Все» of a channel with sections, nowhere I may not post', () => {
    expect(postTarget(scopes, 'premium')).toBe('premium');
    expect(postTarget(scopes, EVERYONE)).toBe(EVERYONE);
    expect(postTarget(scopes, null)).toBeNull();
    expect(postTarget(scopes, 'club')).toBeNull();
    expect(postTarget([], null)).toBe(EVERYONE);
    // Everyone's alone is no sections: the feed posts there.
    expect(postTarget([scope(EVERYONE, '', true)], null)).toBe(EVERYONE);
  });

  it('post only where a posting level lets a reader post, by the core’s word (2026-10-23)', () => {
    const only = [{ ...scope(EVERYONE, '', true), post: false }, { ...scope('premium', 'Премиум', true), post: true }, { ...scope('club', 'Клуб', true), post: false }];
    expect(postTarget(only, 'premium')).toBe('premium');
    expect(postTarget(only, null)).toBeNull();
    expect(postTarget(only, 'club')).toBeNull();
  });

  it('are read from a channel of a core without levels as nothing', () => {
    expect(levelsOf({} as never)).toEqual({ scopes: [], levels: [], my_levels: [] });
  });
});

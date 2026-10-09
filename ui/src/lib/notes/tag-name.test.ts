// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { findTag, normalizeTagName, tagChoice } from './tag-name';

const tags = [
  { name: 'работа', color: '#f26d6d' },
  { name: 'inbox', color: '#6366f1' },
];

describe('normalizeTagName', () => {
  it('trims and lowercases, Cyrillic too, and keeps a hash', () => {
    expect(normalizeTagName('  Работа ')).toBe('работа');
    expect(normalizeTagName('INBOX')).toBe('inbox');
    expect(normalizeTagName('#Plan')).toBe('#plan');
    expect(normalizeTagName('   ')).toBe('');
  });
});

describe('findTag', () => {
  it('finds a tag whatever the case', () => {
    expect(findTag(tags, 'РАБОТА')?.color).toBe('#f26d6d');
    expect(findTag(tags, ' Inbox')?.name).toBe('inbox');
    expect(findTag(tags, 'other')).toBeUndefined();
    expect(findTag(tags, '')).toBeUndefined();
  });
});

describe('tagChoice', () => {
  it('a new name goes lowercase to the new tag and to the note', () => {
    expect(tagChoice('Работа', [], [])).toEqual({ name: 'работа', existing: undefined });
  });

  it('a name in other case is the existing tag: no new tag, its color stays', () => {
    expect(tagChoice('Работа', tags, [])).toEqual({ name: 'работа', existing: tags[0] });
  });

  it('a tag the note has, in any case, or an empty name adds nothing', () => {
    expect(tagChoice('РАБОТА', tags, ['работа'])).toBeNull();
    expect(tagChoice('работа', tags, ['Работа'])).toBeNull();
    expect(tagChoice('  ', tags, [])).toBeNull();
  });
});

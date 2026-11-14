// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { MessengerChat, MessengerContact, MessengerProfile } from '../api';
import { chatMatches, contactHaystacks, contactMatches, excerpt, fold, highlightRanges } from './search';

const PK = 'ab12cd34'.repeat(8);
const NPUB = 'npub1qwertyzxcvbnm7f3k9uq2w5e8r4t6y0u1i3o5p7a9s2d4f6g8h0j';

function contact(over: Partial<MessengerContact> = {}, profile: Partial<MessengerProfile> | null = null): MessengerContact {
  return {
    pubkey: PK, npub: NPUB, nickname: null, note: null, followed: false,
    profile: profile as MessengerProfile | null, created_at: 0, updated_at: 0, ...over,
  };
}

function chat(over: Partial<MessengerChat> = {}): MessengerChat {
  return {
    id: `dm:${PK}`, kind: 'dm', peer_pubkey: PK, peer_npub: NPUB, title: '', picture: null, is_contact: true,
    is_muted: false, unread: 0, last_message_at: null, last_preview: null, pinned: false, archived: false,
    mode: 'full_chat', can_send: true, ...over,
  };
}

const mama = contact({ nickname: 'Мама' }, { display_name: 'Елена Петрова', name: 'lena', nip05: 'alice@example.org' });

describe('fold', () => {
  it('drops case and marks', () => {
    expect(fold('José')).toBe('jose');
    expect(fold('ЁЛКА')).toBe('елка');
  });

  // The same pairs as `messenger_store::search::fold` holds: the index of
  // the conversation folds its words by this rule, in Rust.
  it('folds as the search of the conversation does', () => {
    const pairs: [string, string][] = [
      ['Ёлка', 'елка'], ['Ёлена', 'елена'], ['José', 'jose'], ['Привет', 'привет'], ['Йогурт', 'иогурт'],
      ['ÅNGSTRÖM', 'angstrom'], ['Straße', 'straße'], ['İstanbul', 'istanbul'], ['ΟΔΥΣΣΕΥΣ', 'οδυσσευς'],
    ];
    for (const [from, to] of pairs) expect(fold(from), from).toBe(to);
  });
});

describe('contactMatches', () => {
  it('finds a contact by the nickname and by the profile name the nickname hides', () => {
    expect(contactMatches(mama, 'мама')).toBe(true);
    expect(contactMatches(mama, 'петрова')).toBe(true);
    expect(contactMatches(mama, 'lena')).toBe(true);
  });

  it('ignores case and folds ё to е', () => {
    expect(contactMatches(mama, 'ЕЛЕНА')).toBe(true);
    expect(contactMatches(mama, 'Ёлена')).toBe(true);
  });

  it('ignores accents', () => {
    expect(contactMatches(contact({}, { name: 'José' }), 'jose')).toBe(true);
    expect(contactMatches(contact({}, { name: 'Jose' }), 'josé')).toBe(true);
  });

  it('finds by the NIP-05 address and its local part', () => {
    expect(contactMatches(mama, 'alice@example.org')).toBe(true);
    expect(contactMatches(mama, 'alice')).toBe(true);
  });

  it('finds by a prefix of the hex key and by a piece of the npub', () => {
    expect(contactMatches(mama, PK.slice(0, 10))).toBe(true);
    expect(contactMatches(mama, PK.slice(0, 10).toUpperCase())).toBe(true);
    expect(contactMatches(mama, NPUB.slice(10, 22))).toBe(true);
  });

  it('wants every word', () => {
    expect(contactMatches(mama, 'елена пет')).toBe(true);
    expect(contactMatches(mama, '  елена   пет ')).toBe(true);
    expect(contactMatches(mama, 'елена xyz')).toBe(false);
  });

  it('does not let a word span two fields', () => {
    expect(contactMatches(mama, 'мамаелена')).toBe(false);
  });

  it('finds a contact with no profile by the key only', () => {
    const bare = contact();
    expect(contactMatches(bare, NPUB.slice(5, 15))).toBe(true);
    expect(contactMatches(bare, 'елена')).toBe(false);
  });

  it('matches everything on an empty query', () => {
    expect(contactMatches(mama, '')).toBe(true);
    expect(contactMatches(mama, '   ')).toBe(true);
  });
});

describe('chatMatches', () => {
  const hay = contactHaystacks([mama]);

  it('finds a chat titled by the nickname by the profile name of its contact', () => {
    const c = chat({ title: 'Мама' });
    expect(chatMatches(c, 'петрова', hay)).toBe(true);
    expect(chatMatches(c, 'петрова', new Map())).toBe(false);
    expect(chatMatches(c, 'мама', new Map())).toBe(true);
  });

  it('finds a chat by its peer key and its last message', () => {
    const c = chat({ title: 'x', last_preview: 'Привет, как дела?' });
    expect(chatMatches(c, PK.slice(0, 8), new Map())).toBe(true);
    expect(chatMatches(c, 'как дела', new Map())).toBe(true);
  });

  it('finds a group by its title only', () => {
    const g = chat({ id: 'group:1', kind: 'group', peer_pubkey: null, peer_npub: null, title: 'Семья Петровых' });
    expect(chatMatches(g, 'петровых', hay)).toBe(true);
    expect(chatMatches(g, 'мама', hay)).toBe(false);
  });
});

describe('highlightRanges', () => {
  const marked = (text: string, query: string) => highlightRanges(text, query).map(([a, b]) => text.slice(a, b));

  it('finds a word at the start of a word, in any case', () => {
    expect(marked('Привет, как дела? приветик', 'прив')).toEqual(['Прив', 'прив']);
    expect(marked('Привет', 'ивет')).toEqual([]);
    expect(highlightRanges('abc', '')).toEqual([]);
  });

  it('takes «ё» for «е» and drops marks, both ways', () => {
    expect(marked('Ёлка в лесу', 'елка')).toEqual(['Ёлка']);
    expect(marked('елка в лесу', 'Ёлк')).toEqual(['елк']);
    expect(marked('José и Jose', 'jose')).toEqual(['José', 'Jose']);
  });

  it('keeps the offsets of a text whose letters are written apart from their marks', () => {
    const text = 'Е\u0308лка';
    expect(highlightRanges(text, 'елка')).toEqual([[0, text.length]]);
    // A mark after the last letter found stays with it.
    expect(marked('Приве\u0308т', 'приве')).toEqual(['Приве\u0308']);
  });

  it('marks every word of the query, cut as the index cuts it', () => {
    expect(marked('миру привет', 'прив-мир')).toEqual(['мир', 'прив']);
    // Words that overlap are one range.
    expect(highlightRanges('привет', 'пр прив')).toEqual([[0, 4]]);
  });

  it('counts a character past the basic plane as one', () => {
    expect(marked('😀 Ёлка', 'ёлка')).toEqual(['Ёлка']);
  });
});

describe('excerpt', () => {
  const marks = (e: { text: string; ranges: [number, number][] }) => e.ranges.map(([a, b]) => e.text.slice(a, b));

  it('keeps a short line whole, on one line', () => {
    const e = excerpt('Привет,\n  Ёлка  ждёт', 'елк');
    expect(e.text).toBe('Привет, Ёлка ждёт');
    expect(marks(e)).toEqual(['Ёлк']);
  });

  it('begins a long line at a word a little before the first match', () => {
    const e = excerpt('Сначала очень длинное вступление про всё на свете, а потом наконец ёлка и снова ёлка', 'ёлк', 20);
    expect(e.text.startsWith('…')).toBe(true);
    expect(e.text).toBe('…а потом наконец ёлка и снова ёлка');
    expect(marks(e)).toEqual(['ёлк', 'ёлк']);
  });

  it('starts at the match itself when no word begins before it in reach', () => {
    const e = excerpt(`${'x'.repeat(60)} José`, 'jose', 10);
    expect(e.text).toBe('…José');
    expect(marks(e)).toEqual(['José']);
  });

  it('gives the line unmarked when nothing matches', () => {
    expect(excerpt('nothing here', 'zzz')).toEqual({ text: 'nothing here', ranges: [] });
  });
});

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { MessengerChat, MessengerContact, MessengerProfile } from '../api';
import { chatMatches, contactHaystacks, contactMatches, fold } from './search';

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

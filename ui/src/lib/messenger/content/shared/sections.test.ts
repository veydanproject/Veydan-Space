// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { MessengerMessage } from '../../api';
import { inSection, linkItems } from './sections';

function msg(id: string, over: Partial<MessengerMessage> = {}): MessengerMessage {
  return {
    id, chat_id: 'c', direction: 'in', status: 'received', content_type: 'text', text: null,
    sender_pubkey: 's', reply_to: null, created_at: 1, edited_at: null, deleted: false,
    failure_reason: null, delivered_at: null, read_at: null, seen_by: [], reactions: [], media: null, ...over,
  } as MessengerMessage;
}

const media = (id: string, kind: string, text: string | null = null) =>
  msg(id, { content_type: 'media', text, media: { kind, name: 'n', mime: 'x/y', size: 1 } });

describe('inSection', () => {
  it('puts every kind of attachment where the runtime does', () => {
    const where = (kind: string) => (['visual', 'files', 'links', 'voice'] as const).filter((s) => inSection(media(kind, kind), s));
    expect(where('image')).toEqual(['visual']);
    expect(where('video')).toEqual(['visual']);
    expect(where('file')).toEqual(['files']);
    expect(where('audio')).toEqual(['files']);
    expect(where('voice')).toEqual(['voice']);
    expect(where('circle')).toEqual(['voice']);
  });

  it('finds texts with links, not other texts or lines', () => {
    expect(inSection(msg('a', { text: 'see https://a.example' }), 'links')).toBe(true);
    expect(inSection(msg('b', { text: 'veydan://group/x' }), 'links')).toBe(true);
    expect(inSection(msg('c', { text: 'just words' }), 'links')).toBe(false);
    expect(inSection(msg('d', { content_type: 'system', text: 'https://a.example' }), 'links')).toBe(false);
    expect(inSection(msg('e', { deleted: true, text: 'https://a.example' }), 'links')).toBe(false);
  });

  it('counts a caption with a link among links too', () => {
    const m = media('a', 'image', 'here https://a.example');
    expect(inSection(m, 'visual')).toBe(true);
    expect(inSection(m, 'links')).toBe(true);
    expect(inSection(m, 'files')).toBe(false);
  });
});

describe('linkItems', () => {
  it('lists every link of every message once, in order', () => {
    const a = msg('a', { text: 'https://a.example and https://b.example, again https://a.example' });
    const b = msg('b', { text: 'npub1 is not enough; veydan://group/x' });
    const c = msg('c', { text: 'nothing here' });
    const items = linkItems([a, b, c]);
    expect(items.map((i) => [i.message.id, i.link.text])).toEqual([
      ['a', 'https://a.example'],
      ['a', 'https://b.example'],
      ['b', 'veydan://group/x'],
    ]);
    expect(items[2].link.type).toBe('internal');
  });
});

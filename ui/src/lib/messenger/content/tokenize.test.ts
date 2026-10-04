// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { linksOf, tokenize } from './tokenize';
import type { Segment } from './types';

const GID = 'a'.repeat(64);
const GROUP = `veydan://group/${GID}?t=private&r=wss%3A%2F%2Fr.example&o=${'b'.repeat(64)}&n=Club`;
const NPUB = 'npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6';

/** `[type, text]` of every part. */
const shape = (text: string) => tokenize(text).map((s) => [s.type, s.text]);
const whole = (parts: Segment[]) => parts.map((s) => s.text).join('');

describe('tokenize', () => {
  it('leaves plain text alone', () => {
    expect(tokenize('')).toEqual([]);
    expect(shape('просто текст\nв две строки')).toEqual([['text', 'просто текст\nв две строки']]);
  });

  it('finds links outside', () => {
    expect(shape('see https://example.com/a?b=1#c and http://x.org')).toEqual([
      ['text', 'see '],
      ['external', 'https://example.com/a?b=1#c'],
      ['text', ' and '],
      ['external', 'http://x.org'],
    ]);
  });

  it('finds links inside', () => {
    expect(shape(`join ${GROUP} now`)).toEqual([['text', 'join '], ['internal', GROUP], ['text', ' now']]);
    expect(shape(`${NPUB}`)).toEqual([['internal', NPUB]]);
    expect(shape(`write to nostr:${NPUB}, please`)).toEqual([
      ['text', 'write to '],
      ['internal', `nostr:${NPUB}`],
      ['text', ', please'],
    ]);
  });

  it('never takes one kind for the other', () => {
    const inside = `veydan://group/${GID}?n=https://example.com`;
    const outside = `https://example.com/?next=veydan://group/${GID}`;
    expect(shape(inside)).toEqual([['internal', inside]]);
    expect(shape(outside)).toEqual([['external', outside]]);
    expect(shape(`https://example.com/${NPUB}`)).toEqual([['external', `https://example.com/${NPUB}`]]);

    for (const s of tokenize(`${inside} ${outside} ${GROUP} ${NPUB} https://a.example`)) {
      if (s.type === 'internal') expect(s.link.startsWith('veydan://') || s.link.includes('npub1')).toBe(true);
      if (s.type === 'external') expect(/^https?:\/\//.test(s.url)).toBe(true);
    }
  });

  it('takes no other scheme for a link', () => {
    for (const text of [
      'javascript:alert(1)',
      'file:///etc/passwd',
      'data:text/html,<b>x</b>',
      'ftp://example.com/a',
      'VEYDAN://group/x',
      'veydan:group/x',
      'veydan:/group/x',
      'mailto:a@example.com',
      'tel:+100',
      'nostr:nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5',
    ]) {
      expect(shape(text), text).toEqual([['text', text]]);
    }
  });

  it('keeps the sentence out of the link', () => {
    expect(shape('Смотри: https://example.com/a.')).toEqual([['text', 'Смотри: '], ['external', 'https://example.com/a'], ['text', '.']]);
    expect(shape('(https://example.com/a)!')).toEqual([['text', '('], ['external', 'https://example.com/a'], ['text', ')!']]);
    expect(shape('https://en.wikipedia.org/wiki/Rust_(language).')).toEqual([
      ['external', 'https://en.wikipedia.org/wiki/Rust_(language)'],
      ['text', '.'],
    ]);
    expect(shape('«https://example.com»,')).toEqual([['text', '«'], ['external', 'https://example.com'], ['text', '»,']]);
    expect(shape(`${GROUP}?!`)).toEqual([['internal', GROUP], ['text', '?!']]);
  });

  it('does not find a link inside a word', () => {
    expect(shape('xhttps://example.com')).toEqual([['text', 'xhttps://example.com']]);
    expect(shape(`привет${GROUP}`)).toEqual([['text', `привет${GROUP}`]]);
    expect(shape(`1${NPUB}`)).toEqual([['text', `1${NPUB}`]]);
    expect(shape(`${NPUB}qq`), 'longer than a key').toEqual([['text', `${NPUB}qq`]]);
    expect(shape(NPUB.slice(0, -1)), 'shorter than a key').toEqual([['text', NPUB.slice(0, -1)]]);
  });

  it('takes links that follow each other', () => {
    expect(shape(`https://a.example\n${GROUP}\thttps://b.example`)).toEqual([
      ['external', 'https://a.example'],
      ['text', '\n'],
      ['internal', GROUP],
      ['text', '\t'],
      ['external', 'https://b.example'],
    ]);
  });

  it('leaves what is not an address as text', () => {
    expect(shape('https://')).toEqual([['text', 'https://']]);
    expect(shape('http://?')).toEqual([['text', 'http://?']]);
    const long = `https://example.com/${'a'.repeat(3000)}`;
    expect(shape(long)).toEqual([['text', long]]);
  });

  it('loses nothing of the text', () => {
    for (const text of [
      `a ${GROUP}. b https://x.example/(y)) c ${NPUB}!`,
      'https://a.example,https://b.example',
      `xhttps://a.example ${GROUP}${GROUP}`,
      '...https://example.com...',
    ]) {
      expect(whole(tokenize(text)), text).toBe(text);
    }
  });

  it('lists each link once', () => {
    const parts = tokenize(`https://a.example ${GROUP} https://a.example text ${GROUP} https://b.example`);
    expect(linksOf(parts).map((s) => s.text)).toEqual(['https://a.example', GROUP, 'https://b.example']);
  });
});

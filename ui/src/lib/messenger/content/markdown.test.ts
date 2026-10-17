// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { MdBlock, MdInline } from '../generated/markdown';
import { readingOf, segmentsOf, showsHost, wordsOf } from './markdown';

const plain = { bold: false, italic: false, strike: false };
const t = (text: string): MdInline => ({ kind: 'text', text, style: plain });

describe('segmentsOf', () => {
  it('finds bare links in words and the links of the marks, not in code', () => {
    const blocks: MdBlock[] = [
      { kind: 'para', inlines: [t('see https://a.example/x and '), { kind: 'link', url: 'https://b.example', host: 'b.example', children: [t('b')] }] },
      { kind: 'code', lang: null, text: 'https://not.a.link' },
      { kind: 'quote', blocks: [{ kind: 'para', inlines: [{ kind: 'link', url: 'veydan://group/abc', host: null, children: [t('club')] }] }] },
      { kind: 'para', inlines: [{ kind: 'code', text: 'https://in.code' }] },
    ];
    const links = segmentsOf(blocks).filter((s) => s.type !== 'text').map((s) => [s.type, s.text]);
    expect(links).toEqual([
      ['external', 'https://a.example/x'],
      ['external', 'https://b.example'],
      ['internal', 'veydan://group/abc'],
    ]);
  });

  it('looks into lists and tables', () => {
    const blocks: MdBlock[] = [
      { kind: 'list', ordered: false, start: 1, items: [{ task: null, blocks: [{ kind: 'para', inlines: [t('https://l.example')] }] }] },
      { kind: 'table', head: [[t('https://h.example')]], rows: [[[t('https://r.example')]]] },
    ];
    expect(segmentsOf(blocks).filter((s) => s.type !== 'text').map((s) => s.text)).toEqual(['https://l.example', 'https://h.example', 'https://r.example']);
  });
});

describe('showsHost', () => {
  it('leaves the host out when the words are an address of it', () => {
    expect(showsHost('https://a.example', 'a.example')).toBe(true);
    expect(showsHost('A.example/path', 'a.example')).toBe(true);
    expect(showsHost('a.example:8443', 'a.example')).toBe(true);
  });

  it('shows the host for any other words', () => {
    expect(showsHost('my bank', 'a.example')).toBe(false);
    expect(showsHost('a.example.evil.example', 'evil.example')).toBe(false);
    expect(showsHost('https://bank.example', 'evil.example')).toBe(false);
    expect(showsHost('a.examplex', 'a.example')).toBe(false);
  });
});

describe('wordsOf', () => {
  it('reads styled words and code alike', () => {
    expect(wordsOf([t('a '), { kind: 'code', text: 'b' }, { kind: 'break' }, t('c')])).toBe('a b\nc');
  });
});

describe('readingOf', () => {
  it('reads a Markdown message without its marks, any other as it is', () => {
    const rich: MdBlock[] = [
      { kind: 'heading', level: 1, inlines: [t('Plan')] },
      { kind: 'list', ordered: false, start: 1, items: [{ task: null, blocks: [{ kind: 'para', inlines: [t('one')] }] }] },
      { kind: 'code', lang: null, text: 'x' },
    ];
    expect(readingOf({ text: '# Plan\n\n- one\n\n```\nx\n```', rich })).toBe('Plan\none\nx');
    expect(readingOf({ text: '**plain**', rich: null })).toBe('**plain**');
  });
});

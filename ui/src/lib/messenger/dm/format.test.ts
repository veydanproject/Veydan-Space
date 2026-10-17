// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { applyMark, markOfKey, type Mark } from './format';

/** `[` and `]` stand for the selection, in and out. */
function run(field: string, mark: Mark): string {
  const start = field.indexOf('[');
  const end = field.indexOf(']') - 1;
  const text = field.replace('[', '').replace(']', '');
  const r = applyMark(text, start, end, mark);
  return `${r.text.slice(0, r.start)}[${r.text.slice(r.start, r.end)}]${r.text.slice(r.end)}`;
}

describe('applyMark', () => {
  it('wraps the words and keeps them selected', () => {
    expect(run('say [hello] now', 'bold')).toBe('say **[hello]** now');
    expect(run('say [hello] now', 'italic')).toBe('say *[hello]* now');
    expect(run('say [hello] now', 'strike')).toBe('say ~~[hello]~~ now');
  });

  it('leaves the spaces at the edges outside the mark', () => {
    expect(run('say[ hello ]now', 'bold')).toBe('say **[hello]** now');
  });

  it('marks each line apart', () => {
    expect(run('[- one\n* two\n> three\n## four\n1. five]', 'bold')).toBe('- **[one**\n* **two**\n> **three**\n## **four**\n1. **five]**');
    expect(run('[one\ntwo]', 'bold')).toBe('**[one**\n**two]**');
    expect(run('[one\n\ntwo]', 'italic')).toBe('*[one*\n\n*two]*');
  });

  it('takes the mark off marked words', () => {
    expect(run('say **[hello]** now', 'bold')).toBe('say [hello] now');
    expect(run('say [**hello**] now', 'bold')).toBe('say [hello] now');
    expect(run('say ~~[hello]~~ now', 'strike')).toBe('say [hello] now');
    expect(run('say *[hello]* now', 'italic')).toBe('say [hello] now');
  });

  it('does not take bold for italic', () => {
    expect(run('say **[hello]** now', 'italic')).toBe('say ***[hello]*** now');
    expect(run('say [**hello**] now', 'italic')).toBe('say *[**hello**]* now');
  });

  it('does nothing to nothing but spaces', () => {
    expect(run('a[   ]b', 'bold')).toBe('a[   ]b');
  });

  it('makes code in a line, with more ticks than inside', () => {
    expect(run('run [cargo test] here', 'code')).toBe('run `[cargo test]` here');
    expect(run('a [x`y] b', 'code')).toBe('a ``[x`y]`` b');
    expect(run('a [`x] b', 'code')).toBe('a `` [`x] `` b');
    expect(run('run `[cargo test]` here', 'code')).toBe('run [cargo test] here');
  });

  it('makes a block of code of lines', () => {
    expect(run('look:\n[fn main() {\n}]\nok', 'code')).toBe('look:\n```\n[fn main() {\n}]\n```\nok');
    expect(run('look: [a\nb]', 'code')).toBe('look: \n```\n[a\nb]\n```');
    expect(run('[a\n```\nb]', 'code')).toBe('````\n[a\n```\nb]\n````');
    expect(run('x\n[whole line]\ny', 'code')).toBe('x\n```\n[whole line]\n```\ny');
  });

  it('quotes the lines the selection touches, and takes the quote off', () => {
    expect(run('one\nt[wo\nthr]ee\nfour', 'quote')).toBe('one\n[> two\n> three]\nfour');
    expect(run('[> two\n> three]', 'quote')).toBe('[two\nthree]');
    expect(run('[a\n\nb]', 'quote')).toBe('[> a\n\n> b]');
  });
});

describe('markOfKey', () => {
  const key = (code: string, mods: Partial<KeyboardEvent> = {}) =>
    markOfKey({ key: '', code, ctrlKey: true, metaKey: false, shiftKey: false, altKey: false, ...mods });

  it('knows the shortcuts of a computer', () => {
    expect(key('KeyB')).toBe('bold');
    expect(key('KeyI')).toBe('italic');
    expect(key('KeyX', { shiftKey: true })).toBe('strike');
    expect(key('KeyM', { shiftKey: true })).toBe('code');
    expect(key('KeyB', { ctrlKey: false, metaKey: true })).toBe('bold');
  });

  it('leaves the rest to the field', () => {
    expect(key('KeyX')).toBe(null);
    expect(key('KeyB', { ctrlKey: false })).toBe(null);
    expect(key('KeyB', { altKey: true })).toBe(null);
  });
});

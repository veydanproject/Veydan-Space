// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { JSONContent } from '@tiptap/core';
import { MarkdownManager } from '@tiptap/markdown';
import { noteMarkdownExtensions } from './tiptap-ext';

const mm = new MarkdownManager({ extensions: noteMarkdownExtensions(), markedOptions: { gfm: true, breaks: true } });
const parse = (md: string) => mm.parse(md);
const serialize = (doc: JSONContent) => mm.serialize(doc);

/** Every node of the doc, depth first. */
function nodes(n: JSONContent): JSONContent[] {
  return [n, ...(n.content ?? []).flatMap(nodes)];
}

const text = (n: JSONContent) => nodes(n).map((c) => c.text ?? '').join('');

/** The checked flags of a task list's items. */
const checks = (list: JSONContent | undefined) => (list?.content ?? []).map((i) => i.attrs?.checked);

/** The task list right under the first item of an ordered list. */
function taskListUnderItem(doc: JSONContent, item = 0): JSONContent | undefined {
  const ol = doc.content?.find((n) => n.type === 'orderedList');
  return ol?.content?.[item]?.content?.find((n) => n.type === 'taskList');
}

describe('note markdown: lists under a numbered item', () => {
  const threeSpaces = '1. Пункт\n   - [x] A\n   - [ ] B\n   - [x] C';

  it('reads every checkbox of a task list indented under "1. " (a)', () => {
    const doc = parse(threeSpaces);
    const item = doc.content?.[0]?.content?.[0];
    expect(doc.content?.[0]?.type).toBe('orderedList');
    expect(item?.content?.map((n) => n.type)).toEqual(['paragraph', 'taskList']);
    expect(checks(taskListUnderItem(doc))).toEqual([true, false, true]);
    expect(text(item!)).toBe('ПунктABC');
    expect(nodes(doc).some((n) => (n.text ?? '').includes('['))).toBe(false);
  });

  it('writes the list back byte for byte (b)', () => {
    expect(serialize(parse(threeSpaces))).toBe(threeSpaces);
    const wide = '10. P\n    - [x] A\n    - [ ] B';
    const doc = parse(wide);
    expect(doc.content?.[0]?.attrs?.start).toBe(10);
    expect(checks(taskListUnderItem(doc))).toEqual([true, false]);
    expect(serialize(doc)).toBe(wide);
  });

  it('reads the two-space form the same way (c)', () => {
    expect(parse('1. Пункт\n  - [x] A\n  - [ ] B\n  - [x] C')).toEqual(parse(threeSpaces));
  });

  it('keeps a plain bullet list under a numbered item (d)', () => {
    const md = '1. Пункт\n   - a\n   - b\n   - c';
    const doc = parse(md);
    const bullets = doc.content?.[0]?.content?.[0]?.content?.find((n) => n.type === 'bulletList');
    expect(bullets?.content).toHaveLength(3);
    expect(serialize(doc)).toBe(md);
  });

  it('stops at the end of the list: the next paragraph is read once (e)', () => {
    const md = '1. Пункт\n   - [x] A\n   - [ ] B\n2. Второй\n   - [ ] z\n\nabc';
    const doc = parse(md);
    expect(doc.content?.map((n) => n.type)).toEqual(['orderedList', 'paragraph']);
    expect(doc.content?.[0]?.content).toHaveLength(2);
    expect(checks(taskListUnderItem(doc, 0))).toEqual([true, false]);
    expect(checks(taskListUnderItem(doc, 1))).toEqual([false]);
    expect(text(doc.content![1])).toBe('abc');
    expect(nodes(doc).filter((n) => n.text === 'abc')).toHaveLength(1);
    expect(serialize(doc)).toBe(md);
  });

  it('leaves a nested numbered list as it was (f)', () => {
    const md = '1. A\n   1. x\n   2. y';
    const doc = parse(md);
    const inner = doc.content?.[0]?.content?.[0]?.content?.find((n) => n.type === 'orderedList');
    expect(inner?.content?.map(text)).toEqual(['x', 'y']);
    expect(serialize(doc)).toBe(md);
  });

  it('reads its own output back to the same doc (g)', () => {
    for (const md of [threeSpaces, '10. P\n    - [x] A\n    - [ ] B', '1. Пункт\n   - [x] A\n2. Второй\n   - [ ] z\n\nabc']) {
      const doc = parse(md);
      expect(parse(serialize(doc))).toEqual(doc);
    }
  });

  it('keeps a task list nested under a task item under a numbered item', () => {
    const md = '1. Пункт\n   - [x] A\n     - [ ] A1\n     - [x] A2\n   - [ ] B';
    const doc = parse(md);
    const outer = taskListUnderItem(doc);
    expect(checks(outer)).toEqual([true, false]);
    const inner = outer?.content?.[0]?.content?.find((n) => n.type === 'taskList');
    expect(checks(inner)).toEqual([false, true]);
    expect(text(inner!)).toBe('A1A2');
    expect(parse(serialize(doc))).toEqual(doc);
  });

  it('reads a task list indented 4 under "1. " with every checkbox (h)', () => {
    for (const md of ['1. A\n    - [x] a\n    - [ ] b\n    - [x] c', '1.  A\n    - [x] a\n    - [ ] b\n    - [x] c']) {
      const doc = parse(md);
      expect(doc.content?.map((n) => n.type)).toEqual(['orderedList']);
      expect(checks(taskListUnderItem(doc))).toEqual([true, false, true]);
      expect(text(doc)).toBe('Aabc');
      expect(nodes(doc).some((n) => (n.text ?? '').includes('['))).toBe(false);
      expect(parse(serialize(doc))).toEqual(doc);
    }
    const two = parse('1.  A\n    - [x] a\n    - [ ] b');
    expect(checks(taskListUnderItem(two))).toEqual([true, false]);
    expect(nodes(two).some((n) => (n.text ?? '').includes('['))).toBe(false);
  });

  it('still nests a task item indented deeper than its 2-space parent (i)', () => {
    const doc = parse('1. A\n  - [x] a\n    - [ ] a1');
    const outer = taskListUnderItem(doc);
    expect(checks(outer)).toEqual([true]);
    const inner = outer?.content?.[0]?.content?.find((n) => n.type === 'taskList');
    expect(checks(inner)).toEqual([false]);
    expect(text(inner!)).toBe('a1');
  });

  it('measures the last item by its own lines, not by an indented list later in the note (j)', () => {
    const md = '1. A\n    - [x] a\n    - [ ] b\n\nТекст\n\n- x\n  - y';
    const doc = parse(md);
    expect(doc.content?.map((n) => n.type)).toEqual(['orderedList', 'paragraph', 'bulletList']);
    expect(checks(taskListUnderItem(doc))).toEqual([true, false]);
    expect(nodes(doc).some((n) => (n.text ?? '').includes('['))).toBe(false);
    const nested = doc.content?.[2]?.content?.[0]?.content?.find((n) => n.type === 'bulletList');
    expect(nested?.content?.map(text)).toEqual(['y']);
  });

  it('ends the list at a bullet, heading, fence or break right after it, as upstream does (k)', () => {
    const cases: [string, string[]][] = [
      ['1. A\n   - [x] a\n   - [ ] b\n- c\n  - d', ['orderedList', 'bulletList']],
      ['1. A\n   - [x] a\n   - [ ] b\n# H\n  x', ['orderedList', 'heading', 'paragraph']],
      ['1. A\n   - [x] a\n   - [ ] b\n```\n  x\n```', ['orderedList', 'codeBlock']],
      ['1. A\n   - [x] a\n   - [ ] b\n***\n  x', ['orderedList', 'horizontalRule', 'paragraph']],
    ];
    for (const [md, types] of cases) {
      const doc = parse(md);
      expect(doc.content?.map((n) => n.type), md).toEqual(types);
      expect(checks(taskListUnderItem(doc)), md).toEqual([true, false]);
      expect(nodes(doc).some((n) => (n.text ?? '').includes('[')), md).toBe(false);
      expect(parse(serialize(doc)), md).toEqual(doc);
    }
  });
});

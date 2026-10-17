// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the bubble needs of a Markdown message besides drawing it: the
// links it holds (for their cards, as a plain text's), and whether the
// words of a link already show the host it opens. The runtime parsed the
// text (messenger-richtext::md) and checked every link; nothing here
// decides what a link may be.

import type { MdBlock, MdInline } from '../generated/markdown';
import { tokenize } from './tokenize';
import type { ExternalUrl, InternalLinkText, Segment } from './types';

/** The pieces of a Markdown message with links in them, in order: bare links of its words, and its `[words](address)`. Code holds no links. */
export function segmentsOf(blocks: MdBlock[]): Segment[] {
  const out: Segment[] = [];
  const inline = (list: MdInline[]) => {
    for (const i of list) {
      if (i.kind === 'text') out.push(...tokenize(i.text));
      else if (i.kind === 'link') {
        out.push(i.host === null
          ? { type: 'internal', text: i.url, link: i.url as InternalLinkText }
          : { type: 'external', text: i.url, url: i.url as ExternalUrl });
      }
    }
  };
  const block = (list: MdBlock[]) => {
    for (const b of list) {
      if (b.kind === 'para' || b.kind === 'heading') inline(b.inlines);
      else if (b.kind === 'quote') block(b.blocks);
      else if (b.kind === 'list') b.items.forEach((it) => block(it.blocks));
      else if (b.kind === 'table') [b.head, ...b.rows].forEach((row) => row.forEach(inline));
    }
  };
  block(blocks);
  return out;
}

/** The words of a link as they read. */
export function wordsOf(list: MdInline[]): string {
  return list.map((i) => (i.kind === 'text' || i.kind === 'code' ? i.text : i.kind === 'link' ? wordsOf(i.children) : '\n')).join('');
}

/**
 * Whether the words of a link are an address of the very host it opens
 * (`<https://a.example/x>`), so the host need not stand beside them. Words
 * that only hold the host somewhere (`a.example.evil.example`) do not.
 */
export function showsHost(words: string, host: string): boolean {
  const s = words.trim().toLowerCase().replace(/^https?:\/\//, '');
  return s === host || ['/', ':', '?', '#'].some((c) => s.startsWith(host + c));
}

/** The text of a message as it reads: a Markdown one without its marks (as the runtime's `md::strip`), any other as it is. */
export function readingOf(m: { text: string | null; rich?: MdBlock[] | null }): string | null {
  if (!m.rich?.length) return m.text;
  const lines: string[] = [];
  const block = (list: MdBlock[]) => {
    for (const b of list) {
      if (b.kind === 'para' || b.kind === 'heading') lines.push(wordsOf(b.inlines));
      else if (b.kind === 'code') lines.push(b.text);
      else if (b.kind === 'quote') block(b.blocks);
      else if (b.kind === 'list') b.items.forEach((it) => block(it.blocks));
      else if (b.kind === 'table') [b.head, ...b.rows].forEach((row) => lines.push(row.map(wordsOf).join(' ')));
    }
  };
  block(m.rich);
  return lines.join('\n').trim();
}

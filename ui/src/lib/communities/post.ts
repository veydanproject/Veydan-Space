// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A post read as an article (the redesign of the channels, 2026-10-23):
// its first line is its title, plain words at the top of the card; the
// rest is its text. The title is the page's reading of the words, the wire
// does not know it. In Markdown the marks of the first line go (a heading's
// `#`, a quote's `>`, a list's `-`, bold, links: their words stay), and the
// blocks the runtime parsed lose that line, so the text under the title
// does not say it twice.

import type { FeedItemView } from './api';
import type { MdBlock } from '$lib/messenger/seams/shared';

/** A post's words split: its title (plain, one line) and what follows it (as written: plain or Markdown). */
export interface PostTitle {
  title: string;
  rest: string;
}

/**
 * The first line of a post as its title, the rest as its text. Blank lines
 * before either do not count; a post of one line is a title alone. A first
 * line Markdown makes into no words of a line (a fence of code, a table, a
 * rule) gives no title: the whole is the text.
 */
export function postTitle(text: string | null | undefined, fmt?: string | null): PostTitle {
  const lines = (text ?? '').replace(/\r\n?/g, '\n').split('\n');
  const at = lines.findIndex((l) => l.trim() !== '');
  if (at < 0) return { title: '', rest: '' };
  const md = fmt === 'md';
  const first = lines[at].trim();
  if (md && /^(```|~~~|\||([-*_])(\s*\2){2,}\s*$)/.test(first)) return { title: '', rest: trimLines(lines.slice(at)) };
  let after = lines.slice(at + 1);
  // «Title» over `===` or `---` is a heading in Markdown: its line goes with it.
  if (md && after.length && /^\s*(=+|-+)\s*$/.test(after[0])) after = after.slice(1);
  return { title: md ? plainLine(first) : first, rest: trimLines(after) };
}

/** Lines without the blank ones before and after them, the inside kept as written (indents of code included). */
function trimLines(lines: string[]): string {
  let a = 0;
  let b = lines.length;
  while (a < b && lines[a].trim() === '') a++;
  while (b > a && lines[b - 1].trim() === '') b--;
  return lines.slice(a, b).join('\n');
}

/** One line of Markdown as the words it shows. */
function plainLine(line: string): string {
  let s = line;
  s = s.replace(/^(>\s*)+/, '');
  s = s.replace(/^#{1,6}(\s+|$)/, '').replace(/\s+#+\s*$/, '');
  s = s.replace(/^([-*+]|\d{1,9}[.)])\s+/, '');
  s = s.replace(/^\[[ xX]\]\s+/, '');
  s = s.replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1');
  s = s.replace(/<((?:https?|veydan):[^>\s]+)>/g, '$1');
  s = s.replace(/`([^`]*)`/g, '$1');
  s = s.replace(/(\*\*|__|~~)(.+?)\1/g, '$2');
  s = s.replace(/(^|[^\p{L}\p{N}])([*_])(\S(?:.*?\S)?)\2(?=[^\p{L}\p{N}]|$)/gu, '$1$3');
  s = s.replace(/\\([\\`*_{}[\]()#+\-.!>|~])/g, '$1');
  return s.trim();
}

/**
 * The blocks of a Markdown post without its first line, the one its title
 * shows: a heading goes, a paragraph loses its first line (each line break
 * of a post is a break of its paragraph), a quote or a list loses the
 * first line of its first block or item.
 */
export function dropFirstLine(blocks: readonly MdBlock[]): MdBlock[] {
  if (!blocks.length) return [];
  const [head, ...tail] = blocks;
  const kept = afterFirstLine(head);
  return kept ? [kept, ...tail] : tail;
}

function afterFirstLine(b: MdBlock): MdBlock | null {
  switch (b.kind) {
    case 'heading':
    case 'rule':
      return null;
    case 'para': {
      const k = b.inlines.findIndex((i) => i.kind === 'break');
      const rest = k < 0 ? [] : b.inlines.slice(k + 1);
      return rest.length ? { ...b, inlines: rest } : null;
    }
    case 'quote': {
      const inner = dropFirstLine(b.blocks);
      return inner.length ? { ...b, blocks: inner } : null;
    }
    case 'list': {
      const [first, ...others] = b.items;
      if (!first) return null;
      const inner = dropFirstLine(first.blocks);
      if (inner.length) return { ...b, items: [{ ...first, blocks: inner }, ...others] };
      return others.length ? { ...b, items: others, start: b.start + 1 } : null;
    }
    case 'code': {
      const lines = b.text.split('\n');
      return lines.length > 1 ? { ...b, text: lines.slice(1).join('\n') } : null;
    }
    case 'table':
      return b.rows.length ? { ...b, head: [] } : null;
  }
}

/** When the post was last edited, unix seconds; `null` when never. */
export const editedAt = (item: FeedItemView): number | null => item.edited_at ?? null;

/** A line of text as drawn: its top, its bottom and where its words end, from the top left of the text. */
export interface DrawnLine {
  top: number;
  bottom: number;
  right: number;
}

/**
 * The lines of a text from the boxes of its words (the client rects of its
 * text nodes): boxes that share a line (overlap in height by more than a
 * few pixels) are one line; top to bottom.
 */
export function linesOf(rects: readonly { top: number; bottom: number; right: number }[]): DrawnLine[] {
  const lines: DrawnLine[] = [];
  for (const r of [...rects].sort((a, b) => a.top - b.top)) {
    const l = lines.find((x) => Math.min(x.bottom, r.bottom) - Math.max(x.top, r.top) > Math.min(r.bottom - r.top, x.bottom - x.top) / 2);
    if (l) {
      l.top = Math.min(l.top, r.top);
      l.bottom = Math.max(l.bottom, r.bottom);
      l.right = Math.max(l.right, r.right);
    } else lines.push({ top: r.top, bottom: r.bottom, right: r.right });
  }
  return lines.sort((a, b) => a.top - b.top);
}

/**
 * Where the text in `inner` is cut to show `n` lines (Markdown blocks,
 * lists and quotes included): the height down to the bottom of its `n`-th
 * line, and that line; `null` when it has no more lines than that.
 */
export function cutAt(inner: HTMLElement, n: number): { height: number; line: DrawnLine } | null {
  const origin = inner.getBoundingClientRect();
  const walker = document.createTreeWalker(inner, NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  const rects: DrawnLine[] = [];
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (!node.textContent?.trim()) continue;
    range.selectNodeContents(node);
    for (const r of range.getClientRects()) {
      if (r.width > 0 && r.height > 0) rects.push({ top: r.top - origin.top, bottom: r.bottom - origin.top, right: r.right - origin.left });
    }
  }
  const lines = linesOf(rects);
  if (lines.length <= n) return null;
  const line = lines[n - 1];
  return { height: Math.ceil(line.bottom), line };
}

/** What a card knows when it decides where the menu of a voice message or a circle goes. */
export interface RecordingCardOf {
  comment: boolean;
  title: string;
  deleted: boolean;
  /** The post is one voice message or one circle (not an album, not a picked file). */
  recording: boolean;
  /** The section's name, shown in «Все» in the title's row. */
  section: string | null;
  pinned: boolean;
}

/**
 * A voice message or a circle posted alone has no words, so no title: the
 * card shows the player first, with the menu at the right of its row, and
 * no empty row above it. The row stays when it has something to say (the
 * section in «Все», the pin); a comment keeps its head.
 */
export function recordingCard(c: RecordingCardOf): boolean {
  return c.recording && !c.comment && !c.title && !c.deleted && !c.section && !c.pinned;
}

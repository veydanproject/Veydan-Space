// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** Tiptap extensions for the notes WYSIWYG editor (Markdown stays the storage format). */

import { Node, mergeAttributes, nodeInputRule, type Extensions, type JSONContent } from '@tiptap/core';
import { StarterKit } from '@tiptap/starter-kit';
import { Markdown } from '@tiptap/markdown';
import { markdownDestination } from '$lib/notes/markdown';
import { isEntityBinding } from '$lib/core/bindings';
import { HardBreak } from '@tiptap/extension-hard-break';
import { Paragraph } from '@tiptap/extension-paragraph';
import { Image, type ImageOptions } from '@tiptap/extension-image';
import { OrderedList, ORDERED_LIST_MARKER_PATTERN } from '@tiptap/extension-list';
import { TaskList } from '@tiptap/extension-task-list';
import { TaskItem } from '@tiptap/extension-task-item';
import { TableKit } from '@tiptap/extension-table';
import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';

/** Wiki link delimiters. */
export const WIKI_OPEN = '[[';
export const WIKI_CLOSE = ']]';

/** `[[Target]]` / `[[Target|alias]]` at the start of a string. */
export const WIKI_RE = /^\[\[([^|\]\n]+?)(?:\|([^|\]\n]+?))?\]\]/;

/** Same as `WIKI_RE` but anchored at the end, for the input rule. */
const WIKI_END_RE = /\[\[([^|\]\n]+?)(?:\|([^|\]\n]+?))?\]\]$/;

/** Node attrs from a `WIKI_RE` / `WIKI_END_RE` match. */
const wikiAttrs = (m: RegExpMatchArray) => ({
  target: m[1].trim(),
  label: m[2]?.trim() ?? null,
});

/** Template placeholder delimiters `{{name}}`. */
export const TPL_OPEN = '{{';
export const TPL_CLOSE = '}}';

/** Index of an unclosed `open` (no `close` after it, same line) before the caret, or -1. */
export function unclosedMarkAt(before: string, open: string, close: string): number {
  const at = before.lastIndexOf(open);
  if (at < 0) return -1;
  if (before.indexOf(close, at + open.length) >= 0) return -1;
  if (before.includes('\n', at)) return -1;
  return at;
}

/** Distinct link targets (text before `|`) in a Markdown body. */
export function extractWikiTargets(text: string): string[] {
  const re = /\[\[([^|\]\n]+?)(?:\|[^\]\n]*?)?\]\]/g;
  const out: string[] = [];
  const seen = new Set<string>();
  for (const m of text.matchAll(re)) {
    const target = m[1].trim();
    const key = target.toLowerCase();
    if (target && !seen.has(key)) {
      seen.add(key);
      out.push(target);
    }
  }
  return out;
}

export function wikiMarkup(target: string, label?: string | null): string {
  const inner = label && label !== target ? `${target}|${label}` : target;
  return `${WIKI_OPEN}${inner}${WIKI_CLOSE}`;
}

/** Inline atom for wiki links; round-trips as `[[...]]` in Markdown. */
export const WikiLink = Node.create({
  name: 'wikiLink',
  group: 'inline',
  inline: true,
  atom: true,

  addAttributes() {
    return {
      target: {
        default: '',
        parseHTML: (el) => el.getAttribute('data-target'),
        renderHTML: (attrs) => ({ 'data-target': attrs.target }),
      },
      label: {
        default: null,
        parseHTML: (el) => el.textContent,
        renderHTML: () => ({}),
      },
    };
  },

  parseHTML() {
    return [{ tag: 'a[data-target]' }];
  },

  renderHTML({ node, HTMLAttributes }) {
    // `kind:id` targets are Veydan entity mentions; styled as chips by the editor
    const entity = isEntityBinding(String(node.attrs.target ?? ''));
    return ['a', mergeAttributes(HTMLAttributes, { class: entity ? 'wiki wiki-entity' : 'wiki' }), node.attrs.label ?? node.attrs.target];
  },

  renderText({ node }) {
    return wikiMarkup(node.attrs.target);
  },

  markdownTokenizer: {
    name: 'wikiLink',
    level: 'inline',
    start: (src) => src.indexOf(WIKI_OPEN),
    tokenize(src) {
      const m = WIKI_RE.exec(src);
      if (m) return { type: 'wikiLink', raw: m[0], ...wikiAttrs(m) };
    },
  },

  parseMarkdown: (token, h) =>
    h.createNode('wikiLink', { target: token.target, label: token.label }),

  renderMarkdown: (node: JSONContent) => {
    const { target, label } = node.attrs ?? {};
    return wikiMarkup(String(target ?? ''), label == null ? null : String(label));
  },

  // Typing the closing `]]` after `[[Title` converts the text into a wiki link node
  addInputRules() {
    return [
      nodeInputRule({
        find: WIKI_END_RE,
        type: this.type,
        getAttributes: wikiAttrs,
      }),
    ];
  },
});

export type ResolveSrc = (src: string) => string;

function escAttr(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;');
}

/** Inline image whose `src` attr stays a relative attachment path; display URL is resolved at render. */
export const NoteImage = Image.extend<ImageOptions & { resolveSrc: ResolveSrc }>({
  addOptions() {
    const base = this.parent?.() as ImageOptions;
    return {
      ...base,
      inline: true,
      resolveSrc: (s: string) => s,
      resize: {
        enabled: true,
        directions: ['top-left', 'top-right', 'bottom-left', 'bottom-right'],
        minWidth: 40,
        minHeight: 40,
        alwaysPreserveAspectRatio: true,
      },
    };
  },

  addAttributes() {
    const parent = this.parent?.() ?? {};
    return {
      ...parent,
      src: {
        default: null,
        renderHTML: (attrs: { src?: string | null }) => {
          if (!attrs.src) return {};
          return { src: this.options.resolveSrc(attrs.src) };
        },
      },
    };
  },

  renderHTML({ HTMLAttributes }) {
    const src = this.options.resolveSrc(HTMLAttributes.src ?? '');
    return ['img', mergeAttributes(this.options.HTMLAttributes, HTMLAttributes, { src })];
  },

  renderMarkdown: (node: JSONContent) => {
    const src = String(node.attrs?.src ?? '');
    const alt = String(node.attrs?.alt ?? '');
    const title = String(node.attrs?.title ?? '');
    const width = Number(node.attrs?.width);
    if (width > 0) {
      return `<img src="${escAttr(src)}" alt="${escAttr(alt)}" width="${Math.round(width)}">`;
    }
    const dest = markdownDestination(src);
    return title ? `![${alt}](${dest} "${title}")` : `![${alt}](${dest})`;
  },
});

/** Single newline in Markdown (`breaks: true`) instead of the two-space hard break. */
const SoftBreak = HardBreak.extend({
  renderMarkdown: () => '\n',
});

const EMPTY_PARAGRAPH = new Set(['&nbsp;', '\u00a0']);

/** Paragraph that keeps images inline (stock version lifts a lone image to block level). */
const NoteParagraph = Paragraph.extend({
  parseMarkdown: (token, h) => {
    const content = h.parseInline(token.tokens ?? []);
    const blank = content.length === 1 && content[0].type === 'text' && EMPTY_PARAGRAPH.has(content[0].text ?? '');
    return h.createNode('paragraph', undefined, blank ? [] : content);
  },
});

/** An ordered-list item line: indent, marker, separator (the upstream tokenizer's pattern). */
const ORDERED_ITEM_RE = new RegExp(`^(\\s*)(${ORDERED_LIST_MARKER_PATTERN})([.)])\\s+`);

const stockOrderedTokenizer = OrderedList.config.markdownTokenizer!;

const leadingWs = (line: string) => line.length - line.trimStart().length;

/**
 * Lines that end an ordered list item even without a blank line before them: a copy of
 * PARAGRAPH_INTERRUPTERS in @tiptap/extension-list 3.31.3 (src/ordered-list/utils.ts),
 * which is not exported. Keep it in step with upstream.
 */
const PARAGRAPH_INTERRUPTERS = [
  /^#{1,6}(?:\s|$)/, // heading
  /^[-+*]\s+/, // bullet item
  /^(?:```|~~~)/, // code fence
  /^\$\$/, // block math
  /^(?:(?:-[ \t]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})$/, // thematic break
];
const interruptsLazy = (line: string) => PARAGRAPH_INTERRUPTERS.some((re) => re.test(line));

/**
 * Ordered list that reads back what it writes. The upstream tokenizer (3.31.3) strips
 * indent + marker + 1 from an item's continuation lines, whatever the item's content
 * indent is, so a task list nested under "1. " with 3 spaces (what we write), or with 4
 * (tab size 4 elsewhere), or under "1.  A" keeps stray spaces and only its first checkbox
 * survives; the next save writes the rest as text. Before delegating, take off how far
 * each item's continuation block overshoots that width, so deeper lines keep their
 * relative indent. Drop this once upstream measures the content indent.
 */
const NoteOrderedList = OrderedList.extend({
  markdownTokenizer: {
    ...stockOrderedTokenizer,
    tokenize(src, tokens, lexer) {
      if (!ORDERED_ITEM_RE.test(src)) return stockOrderedTokenizer.tokenize(src, tokens, lexer);
      const lines = src.split('\n');
      const fixed = lines.slice();
      // The items as upstream collects them: every item line, at any indent, starts one;
      // its indented lines up to the next item line are its block; an unindented line
      // after a blank line, or one that opens a heading, bullet, fence, $$ or break, ends
      // the list; any other unindented line is lazy text of the item.
      let i = 0;
      let end = false;
      while (i < lines.length && !end) {
        const m = lines[i].match(ORDERED_ITEM_RE);
        if (!m) break;
        // What upstream strips from this item's continuation lines.
        const width = m[1].length + m[2].length + 1;
        const block: number[] = [];
        let least = Infinity;
        let sawBlank = false;
        let j = i + 1;
        for (; j < lines.length && !ORDERED_ITEM_RE.test(lines[j]); j++) {
          if (!lines[j].trim()) sawBlank = true;
          else if (leadingWs(lines[j]) > 0) {
            block.push(j);
            least = Math.min(least, leadingWs(lines[j]));
          } else if (sawBlank || interruptsLazy(lines[j])) {
            end = true;
            break;
          }
        }
        // How far the whole block sits deeper than that; every line of it has at least this
        // much to spare, and deeper lines keep their relative indent.
        const extra = least - width;
        if (block.length && extra > 0) for (const k of block) fixed[k] = lines[k].slice(extra);
        i = j;
      }
      const tok = stockOrderedTokenizer.tokenize(fixed.join('\n'), tokens, lexer);
      // marked advances by raw.length: give back the original lines, not the shortened ones.
      if (tok && tok.raw !== undefined) tok.raw = lines.slice(0, tok.raw.split('\n').length).join('\n');
      return tok;
    },
  },
});

/** StarterKit with our own paragraph, hard break and ordered list. */
const noteStarterKit = () =>
  StarterKit.configure({ hardBreak: false, paragraph: false, orderedList: false, link: { openOnClick: false } });

/**
 * The nodes that decide how a note's Markdown is read and written, without NodeViews or UI
 * strings, so a test can build a MarkdownManager from them.
 */
export function noteMarkdownExtensions(): Extensions {
  return [noteStarterKit(), NoteOrderedList, SoftBreak, NoteParagraph, TaskList, TaskItem.configure({ nested: true }), TableKit, WikiLink];
}

export function noteExtensions(resolveSrc: ResolveSrc): Extensions {
  return [
    noteStarterKit(),
    NoteOrderedList,
    SoftBreak,
    NoteParagraph,
    NoteImage.configure({ resolveSrc }),
    TaskList,
    // The checkbox is announced in the UI's language (TipTap's default is English).
    TaskItem.configure({
      nested: true,
      a11y: {
        checkboxLabel: (node) =>
          node.textContent ? get(t)('notes_task_checkbox', { text: node.textContent }) : get(t)('notes_task_checkbox_empty'),
      },
    }),
    TableKit,
    WikiLink,
    Markdown.configure({ markedOptions: { gfm: true, breaks: true } }),
  ];
}

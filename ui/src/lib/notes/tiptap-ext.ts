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

export function noteExtensions(resolveSrc: ResolveSrc): Extensions {
  return [
    StarterKit.configure({ hardBreak: false, paragraph: false, link: { openOnClick: false } }),
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

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The extended editor of a post (the redesign of the channels, stage 6):
// a title and a text, joined into the words of the post as one line over
// the rest (the card reads its first line as its title, post.ts), and split
// again when a post is edited. The marks of its bar beyond the composer's
// (headings, lists, a link, a rule), and the draft it keeps while the app
// runs. Pure, but for the draft kept in memory.

import { applyMark, type Mark, type MdBlock } from '$lib/messenger/seams/shared';
import type { FeedItemView } from './api';
import { isPicture, isVideo } from './picked.svelte';

/** What the editor holds: the title (one line), the text under it, and whether the words are Markdown. */
export interface PostDraft {
  title: string;
  body: string;
  md: boolean;
}

/** What begins a Markdown line that is not a paragraph: kept from the title, which is plain words. */
const LEADING_BLOCK = /^(?:[>]|[-+*](?=\s)|\d{1,9}[.)](?=\s))/;

/**
 * The words of a post: the title as its first line, the text under it
 * (an empty title, the text alone; an empty text, the title alone). The
 * title is one line of plain words: a heading's `#` goes, and in Markdown
 * a line that would start a quote or a list is escaped, so it stays a line
 * of words.
 */
export function joinPost(title: string, body: string, md: boolean): string {
  let t = title.replace(/\s*[\r\n]+\s*/g, ' ').trim();
  if (md) {
    t = t.replace(/^#{1,6}(\s+|$)/, '').trim();
    if (LEADING_BLOCK.test(t)) t = `\\${t}`;
  }
  const b = trimBlank(body.replace(/\r\n?/g, '\n'));
  if (!t) return b;
  return b ? `${t}\n${b}` : t;
}

/**
 * A post's words as the editor shows them: its first line the title, the
 * rest the text. In Markdown the title loses a heading's `#` (or the line
 * of a setext heading) and the escape `joinPost` gave it; its other marks
 * stay as written. A first line that is not words of a line (a fence of
 * code, a table, a rule) leaves the title empty.
 */
export function splitPost(text: string | null | undefined, fmt?: string | null): PostDraft {
  const md = fmt === 'md';
  const lines = (text ?? '').replace(/\r\n?/g, '\n').split('\n');
  const at = lines.findIndex((l) => l.trim() !== '');
  if (at < 0) return { title: '', body: '', md };
  const first = lines[at].trim();
  if (md && /^(```|~~~|\||([-*_])(\s*\2){2,}\s*$)/.test(first)) return { title: '', body: trimBlank(lines.slice(at).join('\n')), md };
  let after = lines.slice(at + 1);
  if (md && after.length && /^\s*(=+|-+)\s*$/.test(after[0])) after = after.slice(1);
  const title = md ? first.replace(/^#{1,6}(\s+|$)/, '').replace(/\s+#+\s*$/, '').replace(/^\\(?=[>+*\-\d#])/, '').trim() : first;
  return { title, body: trimBlank(after.join('\n')), md };
}

/** Without the blank lines before and after; the inside as written. */
function trimBlank(s: string): string {
  return s.replace(/^(?:[ \t]*\n)+/, '').replace(/\s+$/, '');
}

// ── The draft handed over between the composer and the editor ──────────

/** The words of the composer as the editor's draft. */
export function draftOfComposer(text: string, md: boolean): PostDraft {
  return { ...splitPost(text, md ? 'md' : null), md };
}

/** The editor's draft back as the composer's words. */
export function composerOfDraft(d: PostDraft): { text: string; md: boolean } {
  return { text: joinPost(d.title, d.body, d.md), md: d.md };
}

/** Nothing written. */
export const isEmptyDraft = (d: PostDraft): boolean => !d.title.trim() && !d.body.trim();

/**
 * The drafts of the editor by chat, while the app runs (as the composer's
 * drafts: kept in memory only). A draft left when the channel was left
 * without closing the editor is taken up again by the next ⤢ of an empty
 * composer.
 */
export class PostDrafts {
  private kept = new Map<string, PostDraft>();

  get(chatId: string): PostDraft | null {
    return this.kept.get(chatId) ?? null;
  }

  set(chatId: string, d: PostDraft) {
    if (isEmptyDraft(d)) this.kept.delete(chatId);
    else this.kept.set(chatId, { ...d });
  }

  drop(chatId: string) {
    this.kept.delete(chatId);
  }

  /**
   * What the editor opens with from the composer: its words, or, when it
   * has none, the draft kept of this chat. Either way the kept one goes:
   * the editor holds it now.
   */
  open(chatId: string, text: string, md: boolean): PostDraft {
    const kept = this.get(chatId);
    this.drop(chatId);
    return !text.trim() && kept ? kept : draftOfComposer(text, md);
  }
}

export const postDrafts: PostDrafts = ((globalThis as Record<string, unknown>).__chanPostDrafts ??= new PostDrafts()) as PostDrafts;

// ── The marks of the editor's bar ───────────────────────────────────────

/** A button of the bar: the composer's marks, and the blocks of a post. */
export type EditorMark = Mark | 'h2' | 'h3' | 'bullets' | 'numbers' | 'link' | 'rule';

/** The field after a mark: its text and what is selected (or where the caret is). */
export interface Marked {
  text: string;
  start: number;
  end: number;
}

const PAIRS: Partial<Record<Mark, string>> = { bold: '**', italic: '*', strike: '~~' };

/**
 * A button of the bar (or its shortcut) on the field: the selected words,
 * or the line of the caret. With nothing selected a mark of words puts its
 * pair at the caret, the caret between them.
 */
export function applyEditorMark(text: string, start: number, end: number, mark: EditorMark): Marked {
  const a = Math.max(0, Math.min(start, end, text.length));
  const b = Math.min(text.length, Math.max(start, end));
  switch (mark) {
    case 'h2':
    case 'h3':
      return heading(text, a, b, mark === 'h2' ? 2 : 3);
    case 'bullets':
    case 'numbers':
      return list(text, a, b, mark === 'numbers');
    case 'link':
      return link(text, a, b);
    case 'rule':
      return rule(text, b);
    case 'quote':
      return lineOf(text, a, b).every((l) => !l.trim()) ? insertAt(text, a, b, '> ', 2, 2) : applyMark(text, a, b, 'quote');
    case 'code':
      if (a === b) return insertAt(text, a, b, '```\n\n```', 4, 4);
      return applyMark(text, a, b, 'code');
    default: {
      if (a !== b) return applyMark(text, a, b, mark);
      const m = PAIRS[mark] ?? '';
      return insertAt(text, a, b, m + m, m.length, m.length);
    }
  }
}

function insertAt(text: string, a: number, b: number, s: string, selFrom: number, selTo: number): Marked {
  return { text: text.slice(0, a) + s + text.slice(b), start: a + selFrom, end: a + selTo };
}

/** The bounds of the lines the selection touches. */
function linesAround(text: string, a: number, b: number): { from: number; to: number } {
  const from = text.lastIndexOf('\n', a - 1) + 1;
  const nl = text.indexOf('\n', b > a && text[b - 1] === '\n' ? b - 1 : b);
  return { from, to: nl < 0 ? text.length : nl };
}

function lineOf(text: string, a: number, b: number): string[] {
  const { from, to } = linesAround(text, a, b);
  return text.slice(from, to).split('\n');
}

/** `## ` before each line touched; lines that all are headings of that level lose it, those of another level change. */
function heading(text: string, a: number, b: number, level: number): Marked {
  const { from, to } = linesAround(text, a, b);
  const lines = text.slice(from, to).split('\n');
  const mark = `${'#'.repeat(level)} `;
  const same = lines.every((l) => !l.trim() || l.startsWith(mark));
  const out = lines.map((l) => {
    const bare = l.replace(/^#{1,6}\s+/, '');
    return same ? bare : l.trim() || lines.length === 1 ? mark + bare : l;
  }).join('\n');
  return lineResult(text, from, to, out, a === b);
}

/** A list of the lines touched: `- ` or `1. `, `2. `…; a list of that kind already goes. */
function list(text: string, a: number, b: number, ordered: boolean): Marked {
  const { from, to } = linesAround(text, a, b);
  const lines = text.slice(from, to).split('\n');
  const marker = ordered ? /^\s*\d{1,9}[.)]\s+/ : /^\s*[-+*]\s+(?!\[)/;
  const any = /^\s*(?:[-+*]|\d{1,9}[.)])\s+/;
  const same = lines.every((l) => !l.trim() || marker.test(l));
  let n = 0;
  const out = lines.map((l) => {
    const bare = l.replace(any, '');
    if (same) return bare;
    if (!l.trim() && lines.length > 1) return l;
    n += 1;
    return `${ordered ? `${n}.` : '-'} ${bare}`;
  }).join('\n');
  return lineResult(text, from, to, out, a === b);
}

/** The lines replaced: the caret at the end of the line when nothing was selected, else the lines selected. */
function lineResult(text: string, from: number, to: number, out: string, caret: boolean): Marked {
  const next = text.slice(0, from) + out + text.slice(to);
  const end = from + out.length;
  return { text: next, start: caret ? end : from, end };
}

/** `[words](https://)`, the address selected to be typed over; nothing selected, the caret between the brackets. */
function link(text: string, a: number, b: number): Marked {
  const words = text.slice(a, b);
  if (!words) return insertAt(text, a, b, '[](https://)', 1, 1);
  const s = `[${words}](https://)`;
  return insertAt(text, a, b, s, words.length + 3, s.length - 1);
}

/** A rule on a line of its own, a blank line around it; the caret after it. */
function rule(text: string, at: number): Marked {
  const before = text.slice(0, at).replace(/[ \t]+$/, '');
  const after = text.slice(at).replace(/^[ \t]+/, '');
  const lead = !before ? '' : before.endsWith('\n\n') ? '' : before.endsWith('\n') ? '\n' : '\n\n';
  const tail = after.startsWith('\n\n') ? '' : after.startsWith('\n') ? '\n' : '\n\n';
  const s = `${lead}---${tail}`;
  const caret = before.length + s.length;
  return { text: before + s + after, start: caret, end: caret };
}

/** A shortcut of the editor on a computer: the composer's, and Ctrl+K for a link. */
export function editorMarkOfKey(e: Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'metaKey' | 'shiftKey' | 'altKey'> & { isComposing?: boolean }): EditorMark | null {
  if (!(e.ctrlKey || e.metaKey) || e.altKey || e.isComposing) return null;
  if (!e.shiftKey && e.code === 'KeyB') return 'bold';
  if (!e.shiftKey && e.code === 'KeyI') return 'italic';
  if (!e.shiftKey && e.code === 'KeyK') return 'link';
  if (e.shiftKey && e.code === 'KeyX') return 'strike';
  if (e.shiftKey && e.code === 'KeyM') return 'code';
  return null;
}

// ── The preview: the post as the feed will draw it ──────────────────────

/** A file of the tray, as the preview draws it. */
export interface PreviewFile {
  key: number;
  name: string;
}

/** What the preview is made of. */
export interface PreviewOf {
  communityId: string;
  /** Who writes: me. */
  author: string;
  /** Now, unix seconds. */
  at: number;
  /** The words joined (`joinPost`), their format, and their blocks once the runtime parsed them. */
  text: string;
  md: boolean;
  rich: MdBlock[] | null;
  /** A new post: the files picked, in order; the words are the caption of the last. */
  files?: readonly PreviewFile[];
  /** An edit: the post (the last of its album) and the parts of the album; its files stay as they are. */
  edited?: FeedItemView | null;
  parts?: readonly FeedItemView[] | null;
}

/** The id of a picked file in the preview: written here, never sent (`isLocal`). */
export const previewId = (key: number) => `local:preview-${key}`;

/**
 * The posts the preview draws, as the feed would get them: a post of
 * words, or one post a file with the words on the last (an album); an
 * edit, the post as it is with the new words. Each is `local:`, so
 * nothing on it can be acted on.
 */
export function previewItems(p: PreviewOf): FeedItemView[] {
  const words = p.text.trim() ? { text: p.text, fmt: p.md ? 'md' : undefined, rich: p.md && p.rich?.length ? p.rich : undefined } : { text: undefined, fmt: undefined, rich: undefined };
  if (p.edited) {
    const parts = p.parts?.length ? p.parts : [p.edited];
    return parts.map((x) => (x.id === p.edited!.id ? { ...x, ...words, edited_at: p.at } : x));
  }
  const base: FeedItemView = {
    id: previewId(0), community_id: p.communityId, post_id: '', author: p.author, created_at: p.at, review: 'ok', hidden: false, deleted: false,
    pinned: false, reactions: [], comments: 0, pending: false,
  };
  const files = p.files ?? [];
  if (!files.length) return [{ ...base, ...words }];
  const batch = files.length > 1 ? 'preview' : undefined;
  return files.map((f, i) => {
    const kind = isPicture(f.name) ? 'image' : isVideo(f.name) ? 'video' : 'file';
    const last = i === files.length - 1;
    return { ...base, id: previewId(f.key), media_json: JSON.stringify({ kind, name: f.name, mime: '', size: 0, batch }), ...(last ? words : {}) };
  });
}

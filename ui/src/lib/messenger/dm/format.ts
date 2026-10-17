// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The marks of the composer's format menu: what a button does to the
// selected part of the field. The field holds the Markdown as it is typed
// (`**bold**`, a fence around code); the message goes as Markdown, and the
// runtime parses it (messenger-richtext::md). Pure: text and selection in,
// text and selection out.

export type Mark = 'bold' | 'italic' | 'strike' | 'code' | 'quote';

/** The field after a mark: its text and what stays selected (the marked words). */
export interface Marked {
  text: string;
  start: number;
  end: number;
}

const WRAP: Record<'bold' | 'italic' | 'strike', string> = { bold: '**', italic: '*', strike: '~~' };
/** What begins a line of a list, a quote or a heading. */
const BLOCK = /^\s*(?:>\s?)*(?:[-*+]\s+(?:\[[ xX]\]\s+)?|\d{1,9}[.)]\s+|#{1,6}\s+)?/;

export function applyMark(text: string, start: number, end: number, mark: Mark): Marked {
  const a = Math.max(0, Math.min(start, end, text.length));
  const b = Math.min(text.length, Math.max(start, end));
  if (mark === 'quote') return quote(text, a, b);
  if (mark === 'code') return text.slice(a, b).includes('\n') || wholeLines(text, a, b) ? fence(text, a, b) : inlineCode(text, a, b);
  return wrap(text, a, b, WRAP[mark]);
}

/** `**`, `*`, `~~` around the words: each line apart (a mark does not cross a line), the spaces at the edges left outside. Again on marked words takes the mark off. */
function wrap(text: string, a: number, b: number, m: string): Marked {
  const unwrapped = unwrap(text, a, b, m);
  if (unwrapped) return unwrapped;
  const lines = text.slice(a, b).split('\n');
  let out = '';
  let firstAt = -1;
  let lastEnd = -1;
  for (const [i, line] of lines.entries()) {
    if (i) out += '\n';
    // The marker of a list, a quote or a heading stays before the mark: `- **one**`.
    const marker = line.match(BLOCK)?.[0] ?? '';
    const body = line.slice(marker.length);
    const lead = body.length - body.trimStart().length;
    const core = body.trim();
    if (!core) {
      out += line;
    } else {
      const head = marker + body.slice(0, lead);
      const tail = body.slice(lead + core.length);
      const at = a + out.length + head.length + m.length;
      if (firstAt < 0) firstAt = at;
      lastEnd = at + core.length;
      out += `${head}${m}${core}${m}${tail}`;
    }
  }
  if (firstAt < 0) return { text, start: a, end: b };
  return { text: text.slice(0, a) + out + text.slice(b), start: firstAt, end: lastEnd };
}

/** The selection is the words of a mark (inside it, or with it): the mark comes off. */
function unwrap(text: string, a: number, b: number, m: string): Marked | null {
  const inner = text.slice(a, b);
  // With the marks selected too.
  if (inner.length >= 2 * m.length && inner.startsWith(m) && inner.endsWith(m) && !lonely(inner, m)) {
    const core = inner.slice(m.length, inner.length - m.length);
    return { text: text.slice(0, a) + core + text.slice(b), start: a, end: a + core.length };
  }
  // The words alone, the marks around them.
  if (text.slice(a - m.length, a) === m && text.slice(b, b + m.length) === m && !partOfLonger(text, a - m.length, b + m.length, m)) {
    return { text: text.slice(0, a - m.length) + inner + text.slice(b + m.length), start: a - m.length, end: b - m.length };
  }
  return null;
}

/** `*` of `**x**` is not an italic mark. */
function lonely(inner: string, m: string): boolean {
  return m === '*' && inner.startsWith('**') && inner.endsWith('**') && !inner.startsWith('***');
}

function partOfLonger(text: string, from: number, to: number, m: string): boolean {
  if (m !== '*') return false;
  return text[from - 1] === '*' && text[to] === '*' && text[from - 2] !== '*';
}

/** The longest run of backticks in `s`. */
function longestTicks(s: string): number {
  let best = 0;
  for (const run of s.match(/`+/g) ?? []) best = Math.max(best, run.length);
  return best;
}

/** Code in a line: more backticks than any run inside, a space where the code itself begins or ends with one. */
function inlineCode(text: string, a: number, b: number): Marked {
  const inner = text.slice(a, b);
  if (!inner.trim()) return { text, start: a, end: b };
  if (text[a - 1] === '`' && text[b] === '`' && text[a - 2] !== '`' && text[b + 1] !== '`' && !inner.includes('`')) {
    return { text: text.slice(0, a - 1) + inner + text.slice(b + 1), start: a - 1, end: b - 1 };
  }
  const ticks = '`'.repeat(longestTicks(inner) + 1);
  const pad = inner.startsWith('`') || inner.endsWith('`') ? ' ' : '';
  const open = ticks + pad;
  return { text: text.slice(0, a) + open + inner + pad + ticks + text.slice(b), start: a + open.length, end: a + open.length + inner.length };
}

/** Whether `a..b` holds whole lines of the field. */
function wholeLines(text: string, a: number, b: number): boolean {
  return b > a && (a === 0 || text[a - 1] === '\n') && (b === text.length || text[b] === '\n') && text.slice(a, b).trim().length > 0 && text.length > b - a;
}

/** A block of code: a fence longer than any run of backticks inside, on lines of its own. */
function fence(text: string, a: number, b: number): Marked {
  const inner = text.slice(a, b).replace(/\n+$/, '');
  const ticks = '`'.repeat(Math.max(3, longestTicks(inner) + 1));
  const before = a > 0 && text[a - 1] !== '\n' ? '\n' : '';
  const rest = text.slice(a + inner.length);
  const after = rest && !rest.startsWith('\n') ? '\n' : '';
  const open = `${before}${ticks}\n`;
  const block = `${open}${inner}\n${ticks}${after}`;
  return { text: text.slice(0, a) + block + rest, start: a + open.length, end: a + open.length + inner.length };
}

/** `> ` before every line the selection touches; lines that all are quoted lose it. */
function quote(text: string, a: number, b: number): Marked {
  const from = text.lastIndexOf('\n', a - 1) + 1;
  const nl = text.indexOf('\n', b > a && text[b - 1] === '\n' ? b - 1 : b);
  const to = nl < 0 ? text.length : nl;
  const lines = text.slice(from, to).split('\n');
  const quoted = lines.every((l) => !l.trim() || /^>\s?/.test(l));
  const out = lines.map((l) => (quoted ? l.replace(/^>\s?/, '') : l.trim() ? `> ${l}` : l)).join('\n');
  return { text: text.slice(0, from) + out + text.slice(to), start: from, end: from + out.length };
}

/** A keyboard shortcut of a mark on a computer. */
export function markOfKey(e: Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'metaKey' | 'shiftKey' | 'altKey'> & { isComposing?: boolean }): Mark | null {
  if (!(e.ctrlKey || e.metaKey) || e.altKey || e.isComposing) return null;
  const key = e.code;
  if (!e.shiftKey && key === 'KeyB') return 'bold';
  if (!e.shiftKey && key === 'KeyI') return 'italic';
  if (e.shiftKey && key === 'KeyX') return 'strike';
  if (e.shiftKey && key === 'KeyM') return 'code';
  return null;
}

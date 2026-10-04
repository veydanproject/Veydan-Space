// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** Pure text transformations for a Markdown textarea. */

export interface EditResult {
  text: string;
  selStart: number;
  selEnd: number;
}

export type InlineAction = 'bold' | 'italic' | 'code' | 'strike';
export type LineAction = 'h1' | 'h2' | 'h3' | 'ul' | 'ol' | 'task' | 'quote';
export type EditAction = InlineAction | LineAction | 'link' | 'codeblock' | 'hr';

const INLINE_MARK: Record<InlineAction, string> = { bold: '**', italic: '_', code: '`', strike: '~~' };
const LINE_PREFIX: Record<LineAction, string> = {
  h1: '# ', h2: '## ', h3: '### ', ul: '- ', ol: '1. ', task: '- [ ] ', quote: '> ',
};
const ANY_PREFIX = /^(\s*)(#{1,6} |[-*+] \[[ xX]\] |[-*+] |\d+\. |> )/;

function lineBounds(text: string, start: number, end: number): [number, number] {
  const ls = text.lastIndexOf('\n', start - 1) + 1;
  const leIdx = text.indexOf('\n', end);
  return [ls, leIdx === -1 ? text.length : leIdx];
}

/** Wrap selection with a marker, or unwrap if already wrapped. */
export function toggleInline(text: string, start: number, end: number, action: InlineAction): EditResult {
  const m = INLINE_MARK[action];
  const before = text.slice(0, start);
  const sel = text.slice(start, end);
  const after = text.slice(end);
  if (before.endsWith(m) && after.startsWith(m)) {
    return { text: before.slice(0, -m.length) + sel + after.slice(m.length), selStart: start - m.length, selEnd: end - m.length };
  }
  if (sel.startsWith(m) && sel.endsWith(m) && sel.length >= m.length * 2) {
    const inner = sel.slice(m.length, -m.length);
    return { text: before + inner + after, selStart: start, selEnd: start + inner.length };
  }
  return { text: before + m + sel + m + after, selStart: start + m.length, selEnd: end + m.length };
}

/** Set (or clear, if already set) a line prefix on every selected line. */
export function toggleLinePrefix(text: string, start: number, end: number, action: LineAction): EditResult {
  const prefix = LINE_PREFIX[action];
  const [ls, le] = lineBounds(text, start, end);
  const lines = text.slice(ls, le).split('\n');
  const allHave = lines.every((l) => l.trimStart().startsWith(prefix.trimEnd()) && ANY_PREFIX.exec(l)?.[2] === prefix);
  let counter = 1;
  const out = lines.map((l) => {
    const m = ANY_PREFIX.exec(l);
    const indent = m?.[1] ?? '';
    const body = m ? l.slice(m[0].length) : l;
    if (allHave) return indent + body;
    const p = action === 'ol' ? `${counter++}. ` : prefix;
    return indent + p + body;
  }).join('\n');
  const newText = text.slice(0, ls) + out + text.slice(le);
  return { text: newText, selStart: ls, selEnd: ls + out.length };
}

/** Insert `[text](url)`; selection becomes link text, cursor lands in url slot. */
export function insertLink(text: string, start: number, end: number): EditResult {
  const sel = text.slice(start, end) || 'link';
  const isUrl = /^https?:\/\//i.test(sel);
  const snippet = isUrl ? `[](${sel})` : `[${sel}](url)`;
  const newText = text.slice(0, start) + snippet + text.slice(end);
  const selStart = isUrl ? start + 1 : start + sel.length + 3;
  const selEnd = isUrl ? selStart : selStart + 3;
  return { text: newText, selStart, selEnd };
}

export function insertCodeBlock(text: string, start: number, end: number): EditResult {
  const sel = text.slice(start, end);
  const snippet = `\n\`\`\`\n${sel}\n\`\`\`\n`;
  const newText = text.slice(0, start) + snippet + text.slice(end);
  const inner = start + 5;
  return { text: newText, selStart: inner, selEnd: inner + sel.length };
}

export function insertHr(text: string, start: number, end: number): EditResult {
  const snippet = '\n\n---\n\n';
  const newText = text.slice(0, start) + snippet + text.slice(end);
  return { text: newText, selStart: start + snippet.length, selEnd: start + snippet.length };
}

export function applyAction(text: string, start: number, end: number, action: EditAction): EditResult {
  switch (action) {
    case 'bold': case 'italic': case 'code': case 'strike':
      return toggleInline(text, start, end, action);
    case 'link': return insertLink(text, start, end);
    case 'codeblock': return insertCodeBlock(text, start, end);
    case 'hr': return insertHr(text, start, end);
    default: return toggleLinePrefix(text, start, end, action);
  }
}

/** Indent / outdent selected lines by two spaces. */
export function shiftIndent(text: string, start: number, end: number, outdent: boolean): EditResult {
  const [ls, le] = lineBounds(text, start, end);
  const lines = text.slice(ls, le).split('\n');
  const out = lines.map((l) => (outdent ? l.replace(/^ {1,2}/, '') : '  ' + l)).join('\n');
  const newText = text.slice(0, ls) + out + text.slice(le);
  const delta = out.length - (le - ls);
  return { text: newText, selStart: Math.max(ls, start + (outdent ? Math.max(-2, delta) : 2)), selEnd: end + delta };
}

/** Enter inside a list item: continue the list, or end it when the item is empty. */
export function continueList(text: string, pos: number): EditResult | null {
  const ls = text.lastIndexOf('\n', pos - 1) + 1;
  const line = text.slice(ls, pos);
  const m = /^(\s*)([-*+]|\d+\.)(\s+\[[ xX]\])?\s+(.*)$/.exec(line);
  if (!m) return null;
  const [, indent, marker, task, body] = m;
  if (!body.trim()) {
    // Empty item: remove marker and leave a blank line
    const newText = text.slice(0, ls) + indent + text.slice(pos);
    return { text: newText, selStart: ls + indent.length, selEnd: ls + indent.length };
  }
  const nextMarker = /^\d+\.$/.test(marker) ? `${parseInt(marker, 10) + 1}.` : marker;
  const insert = `\n${indent}${nextMarker}${task ? ' [ ]' : ''} `;
  const newText = text.slice(0, pos) + insert + text.slice(pos);
  return { text: newText, selStart: pos + insert.length, selEnd: pos + insert.length };
}

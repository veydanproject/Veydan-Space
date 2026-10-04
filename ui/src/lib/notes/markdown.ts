// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

export function wordCount(text: string): number {
  const t = text.trim();
  return t ? t.split(/\s+/).length : 0;
}

/** Percent-encode a Markdown link path. Also encodes ( ) ' so the link ends at the real name. */
export function attachmentHref(relPath: string): string {
  return relPath.split('/').map(encodePathSegment).join('/');
}

/** Wrap a destination that contains spaces or parentheses so `)` does not end the link. */
export function markdownDestination(url: string): string {
  if (/[\s()'"]/.test(url)) return `<${url.replaceAll('<', '').replaceAll('>', '')}>`;
  return url;
}

/**
 * A link `](attachments/.../name (1).png)` is cut at the first `)`.
 * Pull the rest of the filename back inside `<...>`.
 */
export function repairAttachmentLinks(md: string): string {
  const mark = '](attachments/';
  let i = 0;
  let out = '';
  while (i < md.length) {
    const at = md.indexOf(mark, i);
    if (at < 0) {
      out += md.slice(i);
      break;
    }
    out += md.slice(i, at);
    const urlStart = at + 2;
    const close = linkClose(md, urlStart);
    const url = md.slice(urlStart, close);
    out += `](${markdownDestination(url)})`;
    i = close + 1;
  }
  return out;
}

/** Index of the `)` that closes a `(...)` destination, including a `)` inside the filename. */
function linkClose(md: string, urlStart: number): number {
  let depth = 1;
  for (let j = urlStart; j < md.length; j++) {
    const c = md[j];
    if (c === '(') depth++;
    else if (c === ')') {
      depth--;
      if (depth === 0 && !filenameContinues(md, j)) return j;
      if (depth === 0) depth = 1;
    }
  }
  return md.length;
}

/** The `)` just seen belongs to `(1)` in the filename, not to the end of the link. */
function filenameContinues(md: string, closeAt: number): boolean {
  const rest = md.slice(closeAt + 1);
  return /^\.[a-z0-9]+\)/i.test(rest) || /^%20/.test(rest) || /^\(/.test(rest);
}

function encodePathSegment(segment: string): string {
  return encodeURIComponent(segment).replace(/[()']/g, (ch) => `%${ch.charCodeAt(0).toString(16).toUpperCase()}`);
}

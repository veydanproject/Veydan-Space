// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Splits message text into plain parts and links. The scheme alone says
// what a link is: `veydan://` leads inside, `http(s)://` leads outside,
// anything else is text. The result is rendered as text nodes, anchors
// and cards; nothing is ever inserted as HTML.
//
// Only what a link looks like is decided here. What an internal link
// means is decided by the runtime (`messengerApi.links.inspect`).

import type { ExternalUrl, InternalLinkText, LinkSegment, Segment } from './types';

const MAX_LINK_LEN = 2048;

// One pass, alternatives in one expression: parts never overlap, and
// whichever scheme comes first takes everything up to the next space.
const LINK_RE = /(veydan:\/\/[^\s<>"'`]+)|(https?:\/\/[^\s<>"'`]+)|((?:nostr:)?npub1[02-9ac-hj-np-z]{58})/g;

const WORD_CHAR = /[\p{L}\p{N}_]/u;

/** Trailing punctuation belongs to the sentence, not to the link. */
function trimTail(link: string): string {
  let s = link;
  for (;;) {
    const cut = s.replace(/[.,;:!?\]}»…]+$/, '');
    // A bracket closes the link only if the link opened it.
    const next = cut.endsWith(')') && count(cut, ')') > count(cut, '(') ? cut.slice(0, -1) : cut;
    if (next === s) return s;
    s = next;
  }
}

function count(s: string, ch: string): number {
  let n = 0;
  for (const c of s) if (c === ch) n++;
  return n;
}

function external(text: string): ExternalUrl | null {
  try {
    const u = new URL(text);
    if ((u.protocol !== 'https:' && u.protocol !== 'http:') || !u.hostname) return null;
    return text as ExternalUrl;
  } catch {
    return null;
  }
}

export function tokenize(text: string): Segment[] {
  const out: Segment[] = [];
  let last = 0;
  let plain = '';
  const flush = (upTo: number) => {
    plain += text.slice(last, upTo);
    if (plain) out.push({ type: 'text', text: plain });
    plain = '';
  };

  for (const m of text.matchAll(LINK_RE)) {
    const start = m.index ?? 0;
    if (start < last) continue;
    // `xhttps://…`, `1npub1…`: part of a word, not a link.
    if (start > 0 && WORD_CHAR.test(text[start - 1])) continue;

    const key = m[3];
    if (key) {
      const after = text[start + key.length];
      if (after !== undefined && WORD_CHAR.test(after)) continue;
    }
    const raw = key ?? trimTail(m[0]);
    if (raw.length > MAX_LINK_LEN) continue;

    let part: LinkSegment | null = null;
    if (m[1] || key) {
      part = { type: 'internal', text: raw, link: raw as InternalLinkText };
    } else {
      const url = external(raw);
      if (url) part = { type: 'external', text: raw, url };
    }
    if (!part) continue;

    flush(start);
    out.push(part);
    last = start + raw.length;
  }
  flush(text.length);
  return out;
}

/** Links of a text, each once, in the order they appear. */
export function linksOf(segments: Segment[]): LinkSegment[] {
  const seen = new Set<string>();
  const out: LinkSegment[] = [];
  for (const s of segments) {
    if (s.type === 'text' || seen.has(s.text)) continue;
    seen.add(s.text);
    out.push(s);
  }
  return out;
}

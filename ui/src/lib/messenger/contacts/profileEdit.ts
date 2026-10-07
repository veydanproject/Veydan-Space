// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The pure parts of the profile editor, without a DOM: the bio's counter,
// the marks the toolbar puts around a selection, the crop of an avatar and
// whether a form differs from the profile it started from.

import type { Color, CropRect, MessengerProfileInput, SocialLink } from '../api';

/** The length of a text as the runtime counts a bio: Unicode code points, not UTF-16 units. */
export function codePoints(s: string): number {
  let n = 0;
  for (const _ of s) n++;
  return n;
}

// ── Marks around a selection ───────────────────────────────────────────

/** A text and the part of it that is selected (`start` ≤ `end`, UTF-16 offsets as a textarea gives them). */
export interface TextSelection {
  value: string;
  start: number;
  end: number;
}

export type Mark = 'bold' | 'italic' | 'strike' | 'code';

export const MARKS: Record<Mark, string> = { bold: '**', italic: '*', strike: '~~', code: '`' };

const COLOR_OPEN = /\{(red|orange|yellow|green|teal|blue|purple|pink|gray)\}$/;

/**
 * `open` and `close` around the selection. The spaces at its edges stay
 * outside (a mark opens only before a non-space); a selection already
 * between these very marks loses them instead. Nothing selected: the two
 * marks at the caret, the caret between them. The result selects the text
 * inside the marks.
 */
export function wrapSelection(sel: TextSelection, open: string, close: string): TextSelection {
  const { value } = sel;
  let start = Math.max(0, Math.min(sel.start, sel.end, value.length));
  let end = Math.min(value.length, Math.max(sel.start, sel.end));
  // Spaces at the edges of the selection stay where they are.
  while (start < end && /\s/.test(value[start])) start++;
  while (end > start && /\s/.test(value[end - 1])) end--;

  if (start === end) {
    // An empty pair of marks at the caret, unless the caret already stands in one.
    if (value.slice(start - open.length, start) === open && value.slice(start, start + close.length) === close) {
      const v = value.slice(0, start - open.length) + value.slice(start + close.length);
      return { value: v, start: start - open.length, end: start - open.length };
    }
    const v = value.slice(0, start) + open + close + value.slice(start);
    return { value: v, start: start + open.length, end: start + open.length };
  }

  const inner = value.slice(start, end);
  // Marks just outside the selection: take them off.
  if (isWrapped(value, start, end, open, close)) {
    const v = value.slice(0, start - open.length) + inner + value.slice(end + close.length);
    return { value: v, start: start - open.length, end: end - open.length };
  }
  // The selection takes the marks in: take them off.
  if (inner.length >= open.length + close.length && inner.startsWith(open) && inner.endsWith(close) && !(open === '*' && inner.startsWith('**'))) {
    const core = inner.slice(open.length, inner.length - close.length);
    const v = value.slice(0, start) + core + value.slice(end);
    return { value: v, start, end: start + core.length };
  }
  const v = value.slice(0, start) + open + inner + close + value.slice(end);
  return { value: v, start: start + open.length, end: end + open.length };
}

/** `**` around the selection is bold, not italic: a single `*` is wrapped only when it is not half of `**`. */
function isWrapped(value: string, start: number, end: number, open: string, close: string): boolean {
  if (value.slice(start - open.length, start) !== open || value.slice(end, end + close.length) !== close) return false;
  if (open === '*' && close === '*') {
    const before = value[start - 2];
    const after = value[end + 1];
    // `**x**` with x selected: bold, not italic; `***x***` is both.
    if (before === '*' && after === '*') return value[start - 3] === '*' && value[end + 2] === '*';
  }
  return true;
}

/** A style mark (bold, italic, strike, code) around the selection, or off it. */
export function toggleMark(sel: TextSelection, mark: Mark): TextSelection {
  return wrapSelection(sel, MARKS[mark], MARKS[mark]);
}

/**
 * The selection in `color`. A selection already in a colour changes to
 * this one, or loses it when it is this one.
 */
export function applyColor(sel: TextSelection, color: Color): TextSelection {
  const open = `{${color}}`;
  const close = '{/}';
  const { value } = sel;
  let start = Math.min(sel.start, sel.end);
  let end = Math.max(sel.start, sel.end);
  while (start < end && /\s/.test(value[start])) start++;
  while (end > start && /\s/.test(value[end - 1])) end--;
  const before = COLOR_OPEN.exec(value.slice(Math.max(0, start - 9), start));
  if (before && value.slice(end, end + close.length) === close) {
    const was = before[0];
    if (was === open) return wrapSelection({ value, start, end }, open, close);
    const v = value.slice(0, start - was.length) + open + value.slice(start);
    const shift = open.length - was.length;
    return { value: v, start: start + shift, end: end + shift };
  }
  return wrapSelection({ value, start, end }, open, close);
}

// ── The crop of an avatar ──────────────────────────────────────────────

/**
 * Where the square looks at: its center as fractions of the picture, and
 * the zoom (1 = the square as large as the picture's shorter side allows).
 * Free of the viewport's size, so a resize keeps the crop.
 */
export interface Crop {
  cx: number;
  cy: number;
  z: number;
}

/** No more zoom than to a square of this many pixels of the picture (and never more than 8×). */
const MIN_CROP_PX = 64;
const MAX_ZOOM = 8;

/** The largest zoom for a picture of `w` by `h`. */
export function maxZoom(w: number, h: number): number {
  const short = Math.min(w, h);
  if (!(short > 0)) return 1;
  return Math.max(1, Math.min(MAX_ZOOM, short / MIN_CROP_PX));
}

/** The whole shorter side, centered. */
export function initialCrop(): Crop {
  return { cx: 0.5, cy: 0.5, z: 1 };
}

/** The size of the square as fractions of the width and of the height. */
function squareOf(w: number, h: number, z: number): { fw: number; fh: number } {
  const side = Math.min(w, h) / z;
  return { fw: Math.min(1, side / w), fh: Math.min(1, side / h) };
}

const clamp = (v: number, lo: number, hi: number) => (v < lo ? lo : v > hi ? hi : v);

/** The crop held inside the picture: the zoom within its range, the square never past an edge. */
export function clampCrop(c: Crop, w: number, h: number): Crop {
  const z = clamp(Number.isFinite(c.z) ? c.z : 1, 1, maxZoom(w, h));
  const { fw, fh } = squareOf(w, h, z);
  const cx = clamp(Number.isFinite(c.cx) ? c.cx : 0.5, fw / 2, 1 - fw / 2);
  const cy = clamp(Number.isFinite(c.cy) ? c.cy : 0.5, fh / 2, 1 - fh / 2);
  return { cx, cy, z };
}

/** The crop as the runtime takes it: fractions 0..1 of the picture, a square in pixels, inside it. */
export function rectOf(c: Crop, w: number, h: number): CropRect {
  const k = clampCrop(c, w, h);
  const { fw, fh } = squareOf(w, h, k.z);
  const x = clamp(k.cx - fw / 2, 0, 1 - fw);
  const y = clamp(k.cy - fh / 2, 0, 1 - fh);
  return { x, y, w: fw, h: fh };
}

/**
 * How the picture is drawn in a square viewport of `view` CSS pixels: its
 * drawn size and its offset (negative: the picture starts left of / above
 * the viewport).
 */
export function layoutOf(c: Crop, w: number, h: number, view: number): { width: number; height: number; left: number; top: number } {
  const r = rectOf(c, w, h);
  const scale = view / (Math.min(w, h) / clampCrop(c, w, h).z);
  return { width: w * scale, height: h * scale, left: -r.x * w * scale, top: -r.y * h * scale };
}

/** The crop moved with a drag of `dx`, `dy` CSS pixels in a viewport of `view` (the picture follows the finger). */
export function panBy(c: Crop, dx: number, dy: number, w: number, h: number, view: number): Crop {
  if (!(view > 0)) return clampCrop(c, w, h);
  const k = clampCrop(c, w, h);
  const scale = view / (Math.min(w, h) / k.z);
  return clampCrop({ cx: k.cx - dx / (w * scale), cy: k.cy - dy / (h * scale), z: k.z }, w, h);
}

/**
 * The crop zoomed to `z`, the point at `px`, `py` of the viewport (CSS
 * pixels from its top left) staying where it is, as far as the edges allow.
 */
export function zoomAt(c: Crop, z: number, px: number, py: number, w: number, h: number, view: number): Crop {
  if (!(view > 0)) return clampCrop({ ...c, z }, w, h);
  const k = clampCrop(c, w, h);
  const r = rectOf(k, w, h);
  // The picture's point under (px, py), as fractions of the picture.
  const u = r.x + (px / view) * r.w;
  const v = r.y + (py / view) * r.h;
  const z2 = clamp(z, 1, maxZoom(w, h));
  const { fw, fh } = squareOf(w, h, z2);
  const x2 = u - (px / view) * fw;
  const y2 = v - (py / view) * fh;
  return clampCrop({ cx: x2 + fw / 2, cy: y2 + fh / 2, z: z2 }, w, h);
}

// ── The form ───────────────────────────────────────────────────────────

const norm = (v: string | null | undefined) => (v ?? '').trim();

/** The socials as the runtime compares them: platform and handle, trimmed. */
function sameSocials(a: SocialLink[], b: SocialLink[]): boolean {
  return a.length === b.length && a.every((x, i) => x.p === b[i].p && norm(x.h) === norm(b[i].h));
}

/** Whether two forms publish the same profile (spaces at the ends do not count). */
export function sameInput(a: MessengerProfileInput, b: MessengerProfileInput): boolean {
  return norm(a.name) === norm(b.name)
    && norm(a.display_name) === norm(b.display_name)
    && norm(a.about) === norm(b.about)
    && norm(a.website) === norm(b.website)
    && norm(a.nip05) === norm(b.nip05)
    && norm(a.lud16) === norm(b.lud16)
    && sameSocials(a.socials, b.socials);
}

/**
 * A website as it is published: `https://` added to a bare domain;
 * `null` when empty. Throws `website_invalid` for anything but an http(s)
 * address with a host, as the runtime takes it (a bare name with no dot
 * is more likely a typo than a site).
 */
export function websiteOf(input: string | null | undefined): string | null {
  const s = norm(input);
  if (!s) return null;
  const bare = !/^[a-z][a-z0-9+.-]*:/i.test(s);
  // The runtime checks the scheme as written: it goes out in lower case.
  const withScheme = bare ? `https://${s}` : s.replace(/^https?:/i, (m) => m.toLowerCase());
  let url: URL;
  try { url = new URL(withScheme); } catch { throw new Error('website_invalid'); }
  if ((url.protocol !== 'https:' && url.protocol !== 'http:') || !url.hostname || (bare && !url.hostname.includes('.'))
    || url.username || url.password || /\s/.test(s)) {
    throw new Error('website_invalid');
  }
  return withScheme;
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The buttons under a bot's message (sdk/docs/bridge.md, «Buttons under a message» D5, D6,
// §2a): what of the runtime's `kb` is drawn, how its rows are laid out, and
// whether a press still waits for the bot. The runtime has checked the
// keyboard already; what is checked here again is checked so that a
// mistake there never becomes a button that does more than it says.
//
// A button is a label and an opaque `d` handed back to the bot, nothing
// else: no link, no action of the app (the owner, 2026-11-28: no links in
// the first version). `d` is never read here.

import type { KbButton, KbStyle, MessageKeyboard } from '../api';

/** Rows and buttons in a row: the contract's limits (D4). */
export const MAX_ROWS = 8;
export const MAX_IN_ROW = 4;
/** A label longer than this is cut, its last character «…». */
export const LABEL_MAX = 64;
/** A label this long or shorter is short: rows of such go as chips. */
export const SHORT_LABEL = 12;
/** Bytes of `d`. */
export const DATA_MAX = 128;
/** A press without the bot's edit waits this long; then the buttons are live again (D6). */
export const PRESS_WAIT_S = 30;

/** Pressed: `d` goes to the bot. */
export interface KeyButton { label: string; style: KbStyle; d: string }

export type KeyBlock =
  /** One button the width of the card; several of equal widths. */
  | { type: 'row'; buttons: KeyButton[] }
  /** Rows of short labels flowing as chips, wrapped where the card ends. */
  | { type: 'chips'; buttons: KeyButton[] };

const STYLES: readonly KbStyle[] = ['primary', 'quiet', 'danger'];

/** Marks that turn the order of letters (RTLO and the rest): «Отклонить» must not read as another word. */
const BIDI_MARKS = '\u061C\u200E\u200F\u202A-\u202E\u2066-\u2069';
const BIDI = new RegExp(`[${BIDI_MARKS}]`, 'g');
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001F\u007F-\u009F\u2028\u2029]+/g;

/** A label as it is shown: one line of plain words, at most `LABEL_MAX` characters. */
export function labelOf(t: unknown): string {
  if (typeof t !== 'string') return '';
  const flat = t.replace(BIDI, '').replace(CONTROL, ' ').replace(/\s+/g, ' ').trim();
  const chars = [...flat];
  return chars.length > LABEL_MAX ? `${chars.slice(0, LABEL_MAX - 1).join('')}…` : flat;
}

const isShort = (label: string) => [...label].length <= SHORT_LABEL;

function buttonOf(b: KbButton): KeyButton | null {
  if (!b || typeof b !== 'object') return null;
  const label = labelOf(b.t);
  if (!label) return null;
  const style: KbStyle = STYLES.includes(b.style as KbStyle) ? (b.style as KbStyle) : 'quiet';
  // A link is no button of this version: whatever else it holds, it is not drawn.
  if ('url' in b) return null;
  if (typeof b.d !== 'string' || !b.d || new TextEncoder().encode(b.d).length > DATA_MAX) return null;
  return { label, style, d: b.d };
}

/** A row of chips: two buttons or more, each with a short label. */
const chipRow = (row: KeyButton[]) => row.length >= 2 && row.every((b) => isShort(b.label));

/**
 * The keyboard as drawn: rows in the bot's order, a row of one button the
 * card's width, a row of several in equal widths; two rows of short labels
 * or more in a row of the keyboard flow together as chips (a choice of a
 * language). A button that is not one (no label, no `d`, a `d` too long, a
 * `url`) is left out; a row left empty, too.
 */
export function keyboardOf(kb: MessageKeyboard | null | undefined): KeyBlock[] {
  if (!kb || !Array.isArray(kb.rows)) return [];
  const rows: KeyButton[][] = [];
  for (const raw of kb.rows.slice(0, MAX_ROWS)) {
    if (!Array.isArray(raw)) continue;
    const row = raw.slice(0, MAX_IN_ROW).map(buttonOf).filter((b): b is KeyButton => !!b);
    if (row.length) rows.push(row);
  }
  const out: KeyBlock[] = [];
  for (let i = 0; i < rows.length;) {
    let j = i;
    while (j < rows.length && chipRow(rows[j])) j++;
    if (j - i >= 2) {
      out.push({ type: 'chips', buttons: rows.slice(i, j).flat() });
      i = j;
    } else {
      out.push({ type: 'row', buttons: rows[i] });
      i++;
    }
  }
  return out;
}

export interface Press { d: string; at: number }

/**
 * The press that still waits for the bot, or `null` when the buttons are
 * live: younger than `PRESS_WAIT_S`, not older than the bot's last edit
 * (the edit was the answer), and of a button this keyboard has (another
 * keyboard came).
 */
export function waitingPress(press: Press | null | undefined, blocks: KeyBlock[], editedAt: number | null, now: number): Press | null {
  if (!press || typeof press.d !== 'string' || !Number.isFinite(press.at)) return null;
  if (now - press.at >= PRESS_WAIT_S || press.at > now + PRESS_WAIT_S) return null;
  if (editedAt !== null && editedAt >= press.at) return null;
  const has = blocks.some((b) => b.buttons.some((x) => x.d === press.d));
  return has ? press : null;
}

/** The message has buttons to draw: a bot's, incoming, not taken back. */
export function hasKeys(m: { direction: string; deleted: boolean; kb?: MessageKeyboard | null }): boolean {
  return m.direction === 'in' && !m.deleted && keyboardOf(m.kb).length > 0;
}

/** Of two presses, the later one. */
export function laterPress(a: Press | null | undefined, b: Press | null | undefined): Press | null {
  if (!a) return b ?? null;
  if (!b) return a;
  return b.at > a.at ? b : a;
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The look of a section (communities-wire §14.7, the redesign of the
// channels): its colour is one of ten of the palette, kept on the wire as
// its index (0…9) and drawn from the tokens `--sec-<name>` (the fill) and
// `--sec-<name>-on` (a glyph on that fill) of each theme; its icon is a key
// of the app's set, drawn by an icon of Icon.svelte. A key or an index this
// app does not know (another client's, a newer set) falls back to a folder
// and the accent.

import type { ScopeView } from './types';

/** The ten colours of the palette, in the order of their index on the wire. */
export const SECTION_COLORS = ['violet', 'blue', 'cyan', 'teal', 'green', 'amber', 'orange', 'red', 'pink', 'slate'] as const;
export type SectionColor = (typeof SECTION_COLORS)[number];

/** The colour's name by its index; `null` for none or one this app does not know. */
export function colorName(index: number | null | undefined): SectionColor | null {
  return typeof index === 'number' && Number.isInteger(index) && index >= 0 && index < SECTION_COLORS.length ? SECTION_COLORS[index] : null;
}

/** The fill of a section's colour as CSS; `null` for none (the page uses its accent). */
export function sectionColor(index: number | null | undefined): string | null {
  const name = colorName(index);
  return name ? `var(--sec-${name})` : null;
}

/** A glyph drawn on that fill; on no colour, the accent's. */
export function sectionInk(index: number | null | undefined): string {
  const name = colorName(index);
  return name ? `var(--sec-${name}-on)` : '#fff';
}

/**
 * The icons a section may take: the key on the wire (`[a-z0-9-]{1,32}`)
 * and the icon of Icon.svelte that draws it. The keys are Tabler's names,
 * so another client of the same set reads them the same.
 */
export const SECTION_ICONS: readonly { key: string; icon: string }[] = [
  { key: 'speakerphone', icon: 'speakerphone' },
  { key: 'star', icon: 'star' },
  { key: 'shield', icon: 'shield' },
  { key: 'rocket', icon: 'rocket' },
  { key: 'book', icon: 'book' },
  { key: 'calendar', icon: 'calendar' },
  { key: 'music', icon: 'music' },
  { key: 'code', icon: 'code' },
  { key: 'bulb', icon: 'bulb' },
  { key: 'heart', icon: 'heart' },
  { key: 'gift', icon: 'gift' },
  { key: 'flame', icon: 'flame' },
  { key: 'photo', icon: 'image' },
  { key: 'message', icon: 'message-circle' },
  { key: 'lock', icon: 'lock' },
  { key: 'users', icon: 'users' },
  { key: 'news', icon: 'news' },
  { key: 'camera', icon: 'camera' },
  { key: 'help', icon: 'help' },
  { key: 'coin', icon: 'coin' },
];

const BY_KEY = new Map(SECTION_ICONS.map((i) => [i.key, i.icon]));

/** The icon that draws a section: its key's, else a folder (everyone's section: the speaker). */
export function sectionIcon(key: string | null | undefined, everyone = false): string {
  return (key && BY_KEY.get(key)) || (everyone ? 'speakerphone' : 'folder');
}

/** Is this a key of the set (what the editor may send). */
export const knownIcon = (key: string | null | undefined): boolean => !!key && BY_KEY.has(key);

/** The look of a section as the page draws it. */
export interface Look {
  icon: string;
  /** The fill, or `null`: the accent. */
  color: string | null;
  ink: string;
  /** The photo's id (to cache by), when it has one: it takes the icon's place. */
  photo: string | null;
}

export function lookOf(s: Pick<ScopeView, 'scope_id' | 'icon' | 'color' | 'photo'>): Look {
  return { icon: sectionIcon(s.icon, s.scope_id === 'everyone'), color: sectionColor(s.color), ink: sectionInk(s.color), photo: s.photo || null };
}

/** The sections in the owner's order (`order`, then as they came); the core sorts them already, a demo or an old core may not. */
export function ordered<T extends Pick<ScopeView, 'scope_id' | 'order'>>(scopes: readonly T[]): T[] {
  return scopes.map((s, i) => ({ s, i })).sort((a, b) => (a.s.order ?? Infinity) - (b.s.order ?? Infinity) || a.i - b.i).map((x) => x.s);
}

/** A list with one item moved by `by` places (−1 up, +1 down); the same list when it cannot move. */
export function moved<T>(list: readonly T[], index: number, by: number): T[] {
  const to = index + by;
  if (index < 0 || index >= list.length || to < 0 || to >= list.length || by === 0) return [...list];
  const out = [...list];
  const [it] = out.splice(index, 1);
  out.splice(to, 0, it);
  return out;
}

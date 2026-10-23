// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The look of a section: the colour's index to its tokens, the icon's key to
// an icon of the app, the owner's order (communities-wire §14.7).

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { SECTION_COLORS, SECTION_ICONS, colorName, knownIcon, lookOf, moved, ordered, sectionColor, sectionIcon, sectionInk } from './look';

const ICONS = readFileSync(new URL('../../core/Icon.svelte', import.meta.url), 'utf8');
const PALETTE = readFileSync(new URL('../../core/styles/palettes/quiet.css', import.meta.url), 'utf8');

describe('the colour of a section', () => {
  it('is one of ten, by its index on the wire', () => {
    expect(SECTION_COLORS).toHaveLength(10);
    expect(colorName(0)).toBe('violet');
    expect(colorName(5)).toBe('amber');
    expect(colorName(9)).toBe('slate');
    expect(sectionColor(7)).toBe('var(--sec-red)');
    expect(sectionInk(7)).toBe('var(--sec-red-on)');
  });

  it('is none for no index, one out of the palette or not whole: the page takes its accent', () => {
    for (const bad of [undefined, null, -1, 10, 2.5, Number.NaN]) {
      expect(colorName(bad)).toBeNull();
      expect(sectionColor(bad)).toBeNull();
    }
    expect(sectionInk(undefined)).toBe('#fff');
  });

  it('has its fill and its ink declared in both themes', () => {
    for (const theme of ['dark', 'light']) {
      const block = PALETTE.match(new RegExp(`\\[data-theme='${theme}'\\]\\s*\\{([^}]*)\\}`))?.[1] ?? '';
      for (const name of SECTION_COLORS) {
        expect(block, `${theme} --sec-${name}`).toMatch(new RegExp(`--sec-${name}:\\s*#`));
        expect(block, `${theme} --sec-${name}-on`).toMatch(new RegExp(`--sec-${name}-on:\\s*#`));
      }
    }
  });
});

describe('the icon of a section', () => {
  it('draws each key of the set with an icon the app has', () => {
    expect(SECTION_ICONS.length).toBeGreaterThanOrEqual(20);
    expect(new Set(SECTION_ICONS.map((i) => i.key)).size).toBe(SECTION_ICONS.length);
    for (const { key, icon } of SECTION_ICONS) {
      expect(key).toMatch(/^[a-z0-9-]{1,32}$/);
      expect(ICONS, icon).toContain(`'${icon}':`);
      expect(sectionIcon(key)).toBe(icon);
      expect(knownIcon(key)).toBe(true);
    }
  });

  it('is a folder for a key this app does not know or none; the speaker for everyone’s', () => {
    expect(sectionIcon('unicorn')).toBe('folder');
    expect(sectionIcon(undefined)).toBe('folder');
    expect(sectionIcon(null, true)).toBe('speakerphone');
    expect(knownIcon('unicorn')).toBe(false);
    expect(sectionIcon('photo')).toBe('image');
  });

  it('gives the look of a section, a photo before the icon', () => {
    expect(lookOf({ scope_id: 'a', icon: 'star', color: 5, photo: 'f00d' })).toEqual({ icon: 'star', color: 'var(--sec-amber)', ink: 'var(--sec-amber-on)', photo: 'f00d' });
    expect(lookOf({ scope_id: 'everyone' })).toEqual({ icon: 'speakerphone', color: null, ink: '#fff', photo: null });
  });
});

describe('the order of the sections', () => {
  const s = (scope_id: string, order?: number) => ({ scope_id, order });

  it('is the owner’s, then as they came', () => {
    expect(ordered([s('a'), s('b', 1), s('c', 0), s('d')]).map((x) => x.scope_id)).toEqual(['c', 'b', 'a', 'd']);
  });

  it('moves one up or down, and stays at the ends', () => {
    expect(moved(['a', 'b', 'c'], 1, -1)).toEqual(['b', 'a', 'c']);
    expect(moved(['a', 'b', 'c'], 1, 1)).toEqual(['a', 'c', 'b']);
    expect(moved(['a', 'b', 'c'], 0, -1)).toEqual(['a', 'b', 'c']);
    expect(moved(['a', 'b', 'c'], 2, 1)).toEqual(['a', 'b', 'c']);
  });
});

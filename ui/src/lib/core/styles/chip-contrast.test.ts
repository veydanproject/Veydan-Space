// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The text of a chip coloured from data (`.tinted`, base.css) is the colour
// mixed towards --text by --chip-ink; its background is the colour at
// --chip-tint over whatever surface it sits on. This test resolves those
// mixes numerically, with the token values of tokens.css and mobile.css,
// for every colour the pickers offer, the colour tokens a chip takes, the
// extremes and a sweep of the whole RGB cube, on every surface of each
// theme, and asserts WCAG AA (4.5:1). It also finds the largest ink share
// that holds and checks the token is one step under it: lower would lose
// the colour for nothing.

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { NAV_COLORS } from '$lib/core/mobile/nav-colors';

type RGB = [number, number, number];
type Theme = 'light' | 'dark';

const STYLES = new URL('.', import.meta.url);
const LIB = new URL('../../', import.meta.url);
const read = (url: URL) => readFileSync(url, 'utf8');

/** The declarations of a `[data-theme='…'] { … }` block, var() resolved in that block. */
function themeTokens(css: string, theme: Theme): Map<string, string> {
  const out = new Map<string, string>();
  const re = new RegExp(`\\[data-theme='${theme}'\\]\\s*\\{([^}]*)\\}`, 'g');
  for (const block of css.matchAll(re)) {
    for (const m of block[1].replace(/\/\*[\s\S]*?\*\//g, '').matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)) {
      out.set(m[1], m[2].trim());
    }
  }
  return out;
}

const tokens = {
  light: new Map([...themeTokens(read(new URL('tokens.css', STYLES)), 'light'), ...themeTokens(read(new URL('mobile.css', STYLES)), 'light')]),
  dark: new Map([...themeTokens(read(new URL('tokens.css', STYLES)), 'dark'), ...themeTokens(read(new URL('mobile.css', STYLES)), 'dark')]),
};

/** A token's value with var() chains followed; undefined for anything but a colour or a percentage. */
function resolve(theme: Theme, name: string): string | undefined {
  let value = tokens[theme].get(name);
  for (let i = 0; value && i < 8; i++) {
    const ref = /^var\((--[a-z0-9-]+)\)$/.exec(value);
    if (!ref) break;
    value = tokens[theme].get(ref[1]);
  }
  return value;
}

function hex(value: string): RGB {
  let h = value.replace('#', '');
  if (h.length === 3) h = [...h].map((c) => c + c).join('');
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)) as RGB;
}

/** A colour token as RGB over `under` (an rgba() composited, as the browser draws it). */
function color(value: string, under: RGB = [255, 255, 255]): RGB | undefined {
  if (/^#[0-9a-f]{3}([0-9a-f]{3})?$/i.test(value)) return hex(value);
  const m = /^rgba\(\s*(\d+),\s*(\d+),\s*(\d+),\s*([\d.]+)\s*\)$/.exec(value);
  if (m) return mix([+m[1], +m[2], +m[3]], under, +m[4]);
  return undefined;
}

function percent(theme: Theme, name: string): number {
  const value = resolve(theme, name);
  expect(value, `${name} in the ${theme} theme`).toMatch(/^\d+(\.\d+)?%$/);
  return parseFloat(value!) / 100;
}

/** `color-mix(in srgb, a p, b)`, and `a` at alpha p over `b`: the same sum on the encoded channels, rounded as drawn. */
function mix(a: RGB, b: RGB, p: number): RGB {
  return a.map((v, i) => Math.round(v * p + b[i] * (1 - p))) as RGB;
}

function luminance(c: RGB): number {
  const lin = (v: number) => {
    const s = v / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2]);
}

function contrast(a: RGB, b: RGB): number {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}

/** Every `#rrggbb` of the colour lists the pickers offer (`const COLORS = [...]`, `TAG_COLORS`, `PALETTE`), across the UI. */
function pickerPalettes(): string[] {
  const out = new Set<string>(NAV_COLORS);
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) walk(path);
      else if (/\.(svelte|ts)$/.test(name) && !name.endsWith('.test.ts')) {
        for (const list of readFileSync(path, 'utf8').matchAll(/const [A-Z_]*(?:COLORS|PALETTE)\s*=\s*\[([^\]]*)\]/g)) {
          for (const h of list[1].matchAll(/'(#[0-9a-fA-F]{6})'/g)) out.add(h[1].toLowerCase());
        }
      }
    }
  };
  walk(LIB.pathname);
  return [...out];
}

/** The accent and background presets of Settings: an accent is a profile's chip colour, a background is under the chips. */
function themePresets(): { accent: string[]; bg: Record<Theme, string[]> } {
  const source = read(new URL('../desktop/SettingsPage.svelte', STYLES));
  const list = (re: RegExp) => [...(re.exec(source)?.[1] ?? '').matchAll(/'(#[0-9a-fA-F]{6})'/g)].map((m) => m[1]);
  return {
    accent: list(/accent:\s*\[([^\]]*)\]/),
    bg: { dark: list(/bg:\s*\{\s*dark:\s*\[([^\]]*)\]/), light: list(/bg:\s*\{[^}]*light:\s*\[([^\]]*)\]/) },
  };
}

const PALETTE = pickerPalettes();
const PRESETS = themePresets();
const EXTREMES = ['#000000', '#ffffff', '#fef08a', '#fffbeb', '#000080', '#1e1b4b', '#f472b6', '#808080', '#ff0000', '#00ff00', '#0000ff', '#ffff00', '#00ffff'];
/** The colour tokens a chip takes in code (a profile's accent, a workspace with none, a domain's grey) and the category colours. */
const TOKEN_CHIPS = ['--accent', '--success', '--text-2', '--text-3', '--cat-purple', '--cat-blue', '--cat-teal', '--cat-pink', '--danger-text', '--warn-text'];
/** The surfaces a chip can sit on. */
const SURFACES = ['--bg', '--bg-2', '--surface', '--surface-2', '--surface-3', '--surface-hover', '--surface-row-hover', '--surface-drawer', '--surface-drawer-footer', '--m-card', '--m-field', '--m-seg', '--m-tile', '--m-sheet', '--m-hub'];

function cube(): RGB[] {
  const out: RGB[] = [];
  for (let r = 0; r <= 255; r += 51) for (let g = 0; g <= 255; g += 51) for (let b = 0; b <= 255; b += 51) out.push([r, g, b]);
  return out;
}

const surfaceCache = new Map<Theme, RGB[]>();
function surfaces(theme: Theme): RGB[] {
  const hit = surfaceCache.get(theme);
  if (hit) return hit;
  const out: RGB[] = [];
  for (const name of SURFACES) {
    const c = color(resolve(theme, name) ?? '');
    if (c) out.push(c);
  }
  // A selected row (the accent's tint over a card) and the background presets.
  const card = color(resolve(theme, '--surface')!)!;
  out.push(color(resolve(theme, '--accent-bg')!, card)!);
  // A custom accent from Settings rewrites --accent-bg to 15% of itself in
  // both themes (theme.ts); NotesTable's picked row is 12% of --accent.
  const accents = [...PRESETS.accent.map(hex), color(resolve(theme, '--accent')!)!];
  for (const a of accents) out.push(mix(a, card, 0.15), mix(a, card, 0.12));
  for (const h of PRESETS.bg[theme]) out.push(hex(h));
  surfaceCache.set(theme, out);
  return out;
}

const chipCache = new Map<Theme, RGB[]>();
function chipColors(theme: Theme): RGB[] {
  const hit = chipCache.get(theme);
  if (hit) return hit;
  const tokenColors = TOKEN_CHIPS.map((name) => color(resolve(theme, name) ?? '')).filter((c): c is RGB => !!c);
  const out = [...PALETTE, ...PRESETS.accent, ...EXTREMES].map(hex).concat(tokenColors, cube());
  chipCache.set(theme, out);
  return out;
}

/** The lowest contrast of the chip's text on its own background, over every colour and surface. */
function worst(theme: Theme, ink: number, tints: number[]): { ratio: number; chip: RGB; under: RGB; tint: number } {
  const text = color(resolve(theme, '--text')!)!;
  let low = { ratio: Infinity, chip: [0, 0, 0] as RGB, under: [0, 0, 0] as RGB, tint: 0 };
  for (const chip of chipColors(theme)) {
    const fg = mix(chip, text, ink);
    for (const under of surfaces(theme)) {
      for (const tint of tints) {
        const ratio = contrast(fg, mix(chip, under, tint));
        if (ratio < low.ratio) low = { ratio, chip, under, tint };
      }
    }
  }
  return low;
}

describe('a chip coloured from data reads in both themes (.tinted)', () => {
  it('finds the colours the pickers offer and the presets of Settings', () => {
    // If a picker or the presets change shape, the sweep must not silently shrink.
    expect(PALETTE.length).toBeGreaterThanOrEqual(8);
    expect(PRESETS.accent.length).toBeGreaterThan(0);
    expect(PRESETS.bg.dark.length).toBeGreaterThan(0);
    expect(PRESETS.bg.light.length).toBeGreaterThan(0);
  });

  it('base.css draws the chip with these tokens and nothing else', () => {
    const base = read(new URL('base.css', STYLES));
    expect(base).toContain('color: color-mix(in srgb, var(--chip) var(--chip-ink), var(--text));');
    expect(base).toContain('background: color-mix(in srgb, var(--chip) var(--chip-tint), transparent);');
    expect(base).toContain('background: color-mix(in srgb, var(--chip) var(--chip-tint-strong), transparent);');
    // The edge never drops below the neutral chip's outline; the picked one is ringed in the ink.
    expect(base).toContain('border-color: color-mix(in srgb, var(--chip) var(--chip-edge), var(--border-2));');
    expect(base).toContain(".tinted.active[style*='--chip'] { border-color: currentColor; box-shadow: 0 0 0 1px currentColor; }");
  });

  for (const theme of ['light', 'dark'] as Theme[]) {
    it(`keeps 4.5:1 for every colour on every surface (${theme})`, () => {
      const ink = percent(theme, '--chip-ink');
      // 0: `.tinted-text`, the text on the surface itself.
      const tints = [0, percent(theme, '--chip-tint'), percent(theme, '--chip-tint-strong')];
      const low = worst(theme, ink, tints);
      expect(low.ratio, `${theme}: ${low.chip} at ${low.tint} over ${low.under}`).toBeGreaterThanOrEqual(4.5);
    });

    it(`takes the colour's largest safe share, less a step of margin (${theme})`, () => {
      const tints = [0, percent(theme, '--chip-tint'), percent(theme, '--chip-tint-strong')];
      let max = 0;
      for (let step = 0; step <= 100; step++) {
        if (worst(theme, step / 100, tints).ratio >= 4.5) max = step / 100;
        else break;
      }
      const ink = percent(theme, '--chip-ink');
      expect(ink).toBeLessThanOrEqual(max);
      expect(max - ink, `${theme}: the largest safe share is ${max}`).toBeLessThanOrEqual(0.08);
    });

    it(`rings the picked chip at 3:1 or more against the surface (${theme})`, () => {
      // `.active`: border and ring in the ink (currentColor); a raw pale
      // colour on white would be 1.6:1. WCAG 1.4.11, a state of a control.
      const ink = percent(theme, '--chip-ink');
      const text = color(resolve(theme, '--text')!)!;
      let low = Infinity;
      for (const chip of chipColors(theme)) {
        const ring = mix(chip, text, ink);
        for (const under of surfaces(theme)) low = Math.min(low, contrast(ring, under));
      }
      expect(low).toBeGreaterThanOrEqual(3);
    });

    it(`keeps a × or a secondary text faded from the ink at 3:1 or more (${theme})`, () => {
      // Inside a chip they inherit the ink and fade by --chip-fade
      // (NoteContext, TotpAddModal, NoteTagsInput, …), never a fixed --text-*.
      const fade = parseFloat(resolve(theme, '--chip-fade') ?? '');
      expect(fade, `--chip-fade in the ${theme} theme`).toBeGreaterThan(0);
      const ink = percent(theme, '--chip-ink');
      const text = color(resolve(theme, '--text')!)!;
      const tints = [percent(theme, '--chip-tint'), percent(theme, '--chip-tint-strong')];
      let low = { ratio: Infinity, chip: [0, 0, 0] as RGB, under: [0, 0, 0] as RGB };
      for (const chip of chipColors(theme)) {
        const fg = mix(chip, text, ink);
        for (const under of surfaces(theme)) {
          for (const tint of tints) {
            const bg = mix(chip, under, tint);
            const ratio = contrast(mix(fg, bg, fade), bg);
            if (ratio < low.ratio) low = { ratio, chip, under };
          }
        }
      }
      expect(low.ratio, `${theme}: ${low.chip} over ${low.under}`).toBeGreaterThanOrEqual(3);
    });

    it(`keeps the neutral chip readable (${theme})`, () => {
      const fg = color(resolve(theme, '--text-2')!)!;
      const bg = color(resolve(theme, '--surface-2')!)!;
      expect(contrast(fg, bg)).toBeGreaterThanOrEqual(4.5);
    });
  }

  it('the old rule failed: raw pink on its tint in the light theme', () => {
    const pink = hex('#f472b6');
    const white = color(resolve('light', '--surface')!)!;
    expect(contrast(pink, mix(pink, white, 0.16))).toBeLessThan(3);
  });
});

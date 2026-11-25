// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { afterEach, describe, expect, it, vi } from 'vitest';
import { accentFills, contrast, onAccentFor, THEME_DEFAULTS } from './theme';

const rgb = (hex: string) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));

/** An element with only what theme.ts touches: its dataset and its inline custom properties. */
function fakeElement() {
  const props = new Map<string, string>();
  return {
    dataset: {} as Record<string, string>,
    style: {
      setProperty: (k: string, v: string) => props.set(k, v),
      removeProperty: (k: string) => props.delete(k),
    },
    props,
  };
}

async function boot(saved: Record<string, string>) {
  vi.resetModules();
  const root = fakeElement();
  const body = fakeElement();
  const store = new Map(Object.entries(saved));
  vi.stubGlobal('document', { documentElement: root, body });
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => store.set(k, v),
  });
  const mod = await import('./theme');
  return { mod, root, body };
}

afterEach(() => vi.unstubAllGlobals());

describe('the text on an accent of one\'s own', () => {
  // The old dark default, the first preset of Settings, the palette's own two, a pale and a deep one.
  it.each(['#7066c6', '#685dde', '#acaeff', '#f4d35e', '#1e3a8a'])('%s reads at rest, hovered and pressed', (hex) => {
    const on = rgb(onAccentFor(hex));
    for (const fill of accentFills(hex)) expect(contrast(on, fill)).toBeGreaterThanOrEqual(4.5);
  });

  it('is white on a deep fill and indigo on a light one, as the palette has it', () => {
    expect(onAccentFor('#7066c6')).toBe('#ffffff');
    expect(onAccentFor(THEME_DEFAULTS.light.accent)).toBe('#ffffff');
    expect(onAccentFor(THEME_DEFAULTS.dark.accent)).toBe('#24234a');
  });

  it('is set with a saved accent and taken off by the reset', async () => {
    const { mod, root, body } = await boot({
      rb_theme: 'dark',
      rb_theme_custom: JSON.stringify({ dark: { accent: '#7066c6' }, light: {} }),
    });
    for (const el of [root, body]) {
      expect(el.props.get('--on-accent')).toBe('#ffffff');
      expect(el.props.get('--accent-active')).toBe('color-mix(in srgb, #7066c6 82%, black)');
    }
    mod.resetThemeOverrides();
    for (const el of [root, body]) {
      expect(el.props.has('--on-accent')).toBe(false);
      expect(el.props.has('--accent-active')).toBe(false);
      expect(el.props.has('--accent')).toBe(false);
    }
  });
});

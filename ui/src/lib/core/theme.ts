// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { get, writable } from 'svelte/store';

export type Theme = 'dark' | 'light';
/** The colours of the interface (styles/palettes/<name>.css); the theme picks their dark or light half. */
export type Palette = 'quiet';
export type ThemeOverrideKey = 'accent';

export type ThemeOverrides = {
  accent?: string;
};

export type ThemeCustom = {
  dark: ThemeOverrides;
  light: ThemeOverrides;
};

export const PALETTES: readonly Palette[] = ['quiet'];

export const THEME_DEFAULTS: Record<Theme, Required<ThemeOverrides>> = {
  dark: { accent: '#7066c6' },
  light: { accent: '#7066c6' },
};

const THEME_KEY = 'rb_theme';
const PALETTE_KEY = 'rb_palette';
const CUSTOM_KEY = 'rb_theme_custom';
const HEX = /^#[0-9a-f]{6}$/;
const EMPTY_CUSTOM: ThemeCustom = { dark: {}, light: {} };

const ACCENT_VARS = [
  '--accent',
  '--accent-hover',
  '--accent-bg',
  '--accent-border',
  '--accent-grad',
  '--accent-text',
  '--accent-text-2',
  '--accent-text-3',
  '--accent-tint',
  '--accent-tint-border',
  '--color-brand-500',
] as const;

function loadTheme(): Theme {
  if (typeof localStorage !== 'undefined') {
    const saved = localStorage.getItem(THEME_KEY);
    if (saved === 'dark' || saved === 'light') return saved;
  }
  return 'dark';
}

function loadPalette(): Palette {
  if (typeof localStorage !== 'undefined') {
    const saved = localStorage.getItem(PALETTE_KEY);
    if (PALETTES.includes(saved as Palette)) return saved as Palette;
  }
  return 'quiet';
}

function parseHex(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined;
  const hex = value.trim().toLowerCase();
  return HEX.test(hex) ? hex : undefined;
}

/** Only the accent is the user's own; the frame and background colours of earlier versions are left behind. */
function parseOverrides(raw: unknown): ThemeOverrides {
  if (!raw || typeof raw !== 'object') return {};
  const accent = parseHex((raw as Record<string, unknown>).accent);
  return accent ? { accent } : {};
}

function loadCustom(): ThemeCustom {
  if (typeof localStorage === 'undefined') return { ...EMPTY_CUSTOM };
  try {
    const saved = localStorage.getItem(CUSTOM_KEY);
    if (!saved) return { ...EMPTY_CUSTOM };
    const parsed = JSON.parse(saved) as Record<string, unknown>;
    return {
      dark: parseOverrides(parsed.dark),
      light: parseOverrides(parsed.light),
    };
  } catch {
    return { ...EMPTY_CUSTOM };
  }
}

/** A colour of one's own, the way the palette makes its accent: the colour fills (buttons, checks), a lighter step of it in the dark and a deeper one in the light is its text. */
function applyAccent(el: HTMLElement, hex: string, theme: Theme) {
  const dark = theme === 'dark';
  const text = dark ? `color-mix(in srgb, ${hex} 52%, white)` : `color-mix(in srgb, ${hex} 85%, black)`;
  el.style.setProperty('--accent', hex);
  el.style.setProperty('--accent-hover', `color-mix(in srgb, ${hex} 90%, black)`);
  el.style.setProperty('--accent-bg', `color-mix(in srgb, ${hex} ${dark ? 18 : 12}%, transparent)`);
  el.style.setProperty('--accent-border', `color-mix(in srgb, ${dark ? text : hex} ${dark ? 40 : 28}%, transparent)`);
  el.style.setProperty('--accent-grad', hex);
  el.style.setProperty('--accent-text', text);
  el.style.setProperty('--accent-text-2', text);
  el.style.setProperty('--accent-text-3', text);
  el.style.setProperty('--accent-tint', `color-mix(in srgb, ${hex} ${dark ? 20 : 12}%, transparent)`);
  el.style.setProperty('--accent-tint-border', `color-mix(in srgb, ${dark ? text : hex} ${dark ? 30 : 28}%, transparent)`);
  el.style.setProperty('--color-brand-500', hex);
}

function applyCustom(theme: Theme, custom: ThemeCustom) {
  if (typeof document === 'undefined') return;
  const o = custom[theme] ?? {};
  for (const el of [document.documentElement, document.body]) {
    if (o.accent) applyAccent(el, o.accent, theme);
    else ACCENT_VARS.forEach((v) => el.style.removeProperty(v));
  }
}

function persist(key: string, val: string) {
  if (typeof localStorage !== 'undefined') {
    localStorage.setItem(key, val);
  }
}

export function syncAndroidChrome(t: Theme) {
  const bridge = (window as Window & { VeydanChrome?: { setLightBars: (light: boolean) => void } }).VeydanChrome;
  bridge?.setLightBars(t === 'light');
}

function applyAll(t: Theme, p: Palette, custom: ThemeCustom) {
  if (typeof document === 'undefined') return;
  for (const el of [document.documentElement, document.body]) {
    el.dataset.theme = t;
    el.dataset.palette = p;
  }
  applyCustom(t, custom);
  syncAndroidChrome(t);
}

export const theme = writable<Theme>(loadTheme());
export const palette = writable<Palette>(loadPalette());
export const themeCustom = writable<ThemeCustom>(loadCustom());

theme.subscribe((val) => {
  persist(THEME_KEY, val);
  applyAll(val, get(palette), get(themeCustom));
});

palette.subscribe((val) => {
  persist(PALETTE_KEY, val);
  applyAll(get(theme), val, get(themeCustom));
});

themeCustom.subscribe((val) => {
  persist(CUSTOM_KEY, JSON.stringify(val));
  applyAll(get(theme), get(palette), val);
});

export function toggleTheme() {
  theme.update((t) => (t === 'dark' ? 'light' : 'dark'));
}

export function resolvedOverride(custom: ThemeCustom, t: Theme, key: ThemeOverrideKey): string {
  return custom[t][key] ?? THEME_DEFAULTS[t][key];
}

export function hasThemeOverrides(custom: ThemeCustom, t: Theme): boolean {
  return !!custom[t].accent;
}

export function setThemeOverride(key: ThemeOverrideKey, hex: string) {
  const t = get(theme);
  const normalized = parseHex(hex);
  if (!normalized) return;
  themeCustom.update((c) => {
    const next = { ...c[t] };
    if (normalized === THEME_DEFAULTS[t][key]) delete next[key];
    else next[key] = normalized;
    return { ...c, [t]: next };
  });
}

export function resetThemeOverrides() {
  const t = get(theme);
  themeCustom.update((c) => ({ ...c, [t]: {} }));
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { get, writable } from 'svelte/store';

export type Theme = 'dark' | 'light';
export type ThemeOverrideKey = 'chrome' | 'accent' | 'bg';

export type ThemeOverrides = {
  chrome?: string;
  accent?: string;
  bg?: string;
};

export type ThemeCustom = {
  dark: ThemeOverrides;
  light: ThemeOverrides;
};

export const THEME_DEFAULTS: Record<Theme, Required<ThemeOverrides>> = {
  dark: { chrome: '#0b0b10', accent: '#8b7bff', bg: '#08080c' },
  light: { chrome: '#fafafd', accent: '#6d5cf0', bg: '#f4f3f8' },
};

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
  '--shadow-accent',
  '--color-brand-500',
] as const;

function loadTheme(): Theme {
  if (typeof localStorage !== 'undefined') {
    const saved = localStorage.getItem('rb_theme');
    if (saved === 'dark' || saved === 'light') return saved;
  }
  return 'dark';
}

function parseHex(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined;
  const hex = value.trim().toLowerCase();
  return HEX.test(hex) ? hex : undefined;
}

function parseOverrides(raw: unknown): ThemeOverrides {
  if (!raw || typeof raw !== 'object') return {};
  const src = raw as Record<string, unknown>;
  const next: ThemeOverrides = {};
  const chrome = parseHex(src.chrome);
  const accent = parseHex(src.accent);
  const bg = parseHex(src.bg);
  if (chrome) next.chrome = chrome;
  if (accent) next.accent = accent;
  if (bg) next.bg = bg;
  return next;
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

function applyAccent(el: HTMLElement, hex: string, theme: Theme) {
  const textMix = theme === 'dark' ? 'white' : 'black';
  el.style.setProperty('--accent', hex);
  el.style.setProperty('--accent-hover', `color-mix(in srgb, ${hex} 82%, black)`);
  el.style.setProperty('--accent-bg', `color-mix(in srgb, ${hex} 15%, transparent)`);
  el.style.setProperty('--accent-border', `color-mix(in srgb, ${hex} 45%, transparent)`);
  el.style.setProperty('--accent-grad', `linear-gradient(135deg, color-mix(in srgb, ${hex} 80%, white), ${hex})`);
  el.style.setProperty('--accent-text', `color-mix(in srgb, ${hex} 72%, ${textMix})`);
  el.style.setProperty('--accent-text-2', `color-mix(in srgb, ${hex} 64%, ${textMix})`);
  el.style.setProperty('--accent-text-3', `color-mix(in srgb, ${hex} 56%, ${textMix})`);
  el.style.setProperty('--accent-tint', `color-mix(in srgb, ${hex} 12%, transparent)`);
  el.style.setProperty('--accent-tint-border', `color-mix(in srgb, ${hex} 30%, transparent)`);
  el.style.setProperty('--shadow-accent', `0 5px 14px color-mix(in srgb, ${hex} 33%, transparent)`);
  el.style.setProperty('--color-brand-500', hex);
}

function applyCustom(theme: Theme, custom: ThemeCustom) {
  if (typeof document === 'undefined') return;
  const roots = [document.documentElement, document.body];
  const o = custom[theme] ?? {};

  for (const el of roots) {
    if (o.chrome) el.style.setProperty('--chrome', o.chrome);
    else el.style.removeProperty('--chrome');

    if (o.bg) el.style.setProperty('--bg', o.bg);
    else el.style.removeProperty('--bg');

    if (o.accent) applyAccent(el, o.accent, theme);
    else ACCENT_VARS.forEach((v) => el.style.removeProperty(v));
  }
}

function persistTheme(val: Theme) {
  if (typeof localStorage !== 'undefined') {
    localStorage.setItem('rb_theme', val);
  }
}

function persistCustom(val: ThemeCustom) {
  if (typeof localStorage !== 'undefined') {
    localStorage.setItem(CUSTOM_KEY, JSON.stringify(val));
  }
}

export function syncAndroidChrome(t: Theme) {
  const bridge = (window as Window & { VeydanChrome?: { setLightBars: (light: boolean) => void } }).VeydanChrome;
  bridge?.setLightBars(t === 'light');
}

function applyAll(t: Theme, custom: ThemeCustom) {
  if (typeof document === 'undefined') return;
  document.documentElement.dataset.theme = t;
  document.body.dataset.theme = t;
  applyCustom(t, custom);
  syncAndroidChrome(t);
}

export const theme = writable<Theme>(loadTheme());
export const themeCustom = writable<ThemeCustom>(loadCustom());

theme.subscribe((val) => {
  persistTheme(val);
  applyAll(val, get(themeCustom));
});

themeCustom.subscribe((val) => {
  persistCustom(val);
  applyAll(get(theme), val);
});

export function toggleTheme() {
  theme.update((t) => (t === 'dark' ? 'light' : 'dark'));
}

export function resolvedOverride(custom: ThemeCustom, t: Theme, key: ThemeOverrideKey): string {
  return custom[t][key] ?? THEME_DEFAULTS[t][key];
}

export function hasThemeOverrides(custom: ThemeCustom, t: Theme): boolean {
  const o = custom[t];
  return !!(o.chrome || o.accent || o.bg);
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

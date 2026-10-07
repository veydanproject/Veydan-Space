// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The languages of the UI besides English and Russian (platform-spec 11.8):
// each comes as files beside the dictionaries, and in this product it has
// every key the English has, desktop and phone, with the same placeholders.
// The files of every source, the web clipper's, the pushes' and the taglines
// included, are checked across the monorepo by scripts/i18n.mjs
// (scripts/tests/i18n.tests.mjs).

import { describe, expect, it } from 'vitest';
import { get } from 'svelte/store';
import { LANGUAGES, dictionary, isLocale, loadLocaleFiles, loadedLocales, locale, localeTag, systemLocale, t } from './i18n';
import { mobileDictionary, t as mobileT } from './mobile/i18n';

const others = LANGUAGES.filter((l) => l.code !== 'en' && l.code !== 'ru');
const braces = (s: string) => [...new Set(s.match(/\{[a-zA-Z_]\w*\}/g) ?? [])].sort().join();

describe('the list of languages', () => {
  it('has English first, each code once, a valid Intl tag and its own name', () => {
    expect(LANGUAGES[0].code).toBe('en');
    expect(new Set(LANGUAGES.map((l) => l.code)).size).toBe(LANGUAGES.length);
    for (const l of LANGUAGES) {
      // The code is what the backend accepts (crates/shell settings.rs, is_locale_code).
      expect(l.code, l.code).toMatch(/^[a-z]{2,3}(-[A-Z]{2})?$/);
      expect(Intl.getCanonicalLocales(l.tag), l.code).toEqual([l.tag]);
      expect(l.native.trim(), l.code).not.toBe('');
      expect(localeTag(l.code)).toBe(l.tag);
    }
  });

  it('knows its codes and nothing else', () => {
    expect(isLocale('ru')).toBe(true);
    expect(isLocale('xx')).toBe(false);
    expect(isLocale(null)).toBe(false);
  });
});

describe('the language of the system', () => {
  const has = (code: string) => LANGUAGES.some((l) => l.code === code);

  it('is taken by its tag or by its language, the first the UI has', () => {
    expect(systemLocale(['ru-RU'])).toBe('ru');
    expect(systemLocale(['xx-YY', 'ru'])).toBe('ru');
    expect(systemLocale([])).toBe('en');
    expect(systemLocale(['xx'])).toBe('en');
    if (has('de')) expect(systemLocale(['de-AT'])).toBe('de');
    if (has('pt-BR')) expect(systemLocale(['pt-PT'])).toBe('pt-BR');
    if (has('pt-BR')) expect(systemLocale(['pt_br'])).toBe('pt-BR');
    if (has('id')) expect(systemLocale(['in-ID'])).toBe('id');
  });

  it('is simplified Chinese only for a reader of it', () => {
    if (!has('zh-CN')) return;
    expect(systemLocale(['zh-CN'])).toBe('zh-CN');
    expect(systemLocale(['zh-Hans-SG'])).toBe('zh-CN');
    expect(systemLocale(['zh-TW', 'en-US'])).toBe('en');
    expect(systemLocale(['zh-Hant-HK'])).toBe('en');
  });
});

describe.each(others.map((l) => l.code))('the language %s in this product', (code) => {
  it('has every key of the English, desktop and phone, with the same placeholders', async () => {
    await loadLocaleFiles(code);
    const files = get(loadedLocales)[code];
    expect(files, 'its files loaded').toBeDefined();
    for (const [layer, en, have] of [
      ['desktop', dictionary.en as Record<string, string>, files.desktop],
      ['mobile', mobileDictionary.en as Record<string, string>, files.mobile],
    ] as const) {
      expect(Object.keys(have).sort(), layer).toEqual(Object.keys(en).sort());
      for (const [k, v] of Object.entries(en)) {
        expect(have[k]?.trim(), `${layer}.${k}`).toBeTruthy();
        expect(braces(have[k]), `${layer}.${k}`).toBe(braces(v));
      }
    }
  });

  it('is what t() shows, on a computer and on a phone', async () => {
    await loadLocaleFiles(code);
    const files = get(loadedLocales)[code];
    locale.set(code);
    try {
      const k = 'common_cancel';
      expect((get(t) as (key: string) => string)(k)).toBe(files.desktop[k]);
      const mk = Object.keys(mobileDictionary.en).find((key) => !files.mobile[key].includes('{'));
      if (mk) expect((get(mobileT) as (key: string) => string)(mk)).toBe(files.mobile[mk]);
    } finally {
      locale.set('en');
    }
  });
});

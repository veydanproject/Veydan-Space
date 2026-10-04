// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

export interface LocaleOption {
  locale: string;
  languages: string;
  label: string;
}

export const LOCALES: LocaleOption[] = [
  { locale: 'en-US', languages: 'en-US,en',       label: 'English (US)' },
  { locale: 'en-GB', languages: 'en-GB,en',       label: 'English (UK)' },
  { locale: 'ru-RU', languages: 'ru-RU,ru',       label: 'Русский' },
  { locale: 'de-DE', languages: 'de-DE,de',       label: 'Deutsch' },
  { locale: 'fr-FR', languages: 'fr-FR,fr',       label: 'Français' },
  { locale: 'es-ES', languages: 'es-ES,es',       label: 'Español (España)' },
  { locale: 'es-MX', languages: 'es-MX,es',       label: 'Español (México)' },
  { locale: 'pt-BR', languages: 'pt-BR,pt',       label: 'Português (Brasil)' },
  { locale: 'pt-PT', languages: 'pt-PT,pt',       label: 'Português (Portugal)' },
  { locale: 'it-IT', languages: 'it-IT,it',       label: 'Italiano' },
  { locale: 'nl-NL', languages: 'nl-NL,nl',       label: 'Nederlands' },
  { locale: 'pl-PL', languages: 'pl-PL,pl',       label: 'Polski' },
  { locale: 'tr-TR', languages: 'tr-TR,tr',       label: 'Türkçe' },
  { locale: 'uk-UA', languages: 'uk-UA,uk',       label: 'Українська' },
  { locale: 'zh-CN', languages: 'zh-CN,zh',       label: '中文 (简体)' },
  { locale: 'zh-TW', languages: 'zh-TW,zh',       label: '中文 (繁體)' },
  { locale: 'ja-JP', languages: 'ja-JP,ja',       label: '日本語' },
  { locale: 'ko-KR', languages: 'ko-KR,ko',       label: '한국어' },
  { locale: 'ar-SA', languages: 'ar-SA,ar',       label: 'العربية' },
  { locale: 'fa-IR', languages: 'fa-IR,fa',       label: 'فارسی' },
  { locale: 'hi-IN', languages: 'hi-IN,hi',       label: 'हिन्दी' },
  { locale: 'id-ID', languages: 'id-ID,id',       label: 'Bahasa Indonesia' },
  { locale: 'vi-VN', languages: 'vi-VN,vi',       label: 'Tiếng Việt' },
  { locale: 'th-TH', languages: 'th-TH,th',       label: 'ภาษาไทย' },
];

export function getLocaleOption(locale: string): LocaleOption | undefined {
  return LOCALES.find((l) => l.locale === locale);
}

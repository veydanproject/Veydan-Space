// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The light entry: the dictionary alone, no stores on the import path of i18n.
// The messenger keeps the phone's keys in its one dictionary.
import { messengerTranslations } from '../i18n';

export const translations = {
  en: messengerTranslations.en,
  ru: messengerTranslations.ru,
  mobile: { en: {}, ru: {} },
} as const;

/** The other languages (core/i18n.ts fetches a language's file when it is first shown). */
export const locales = import.meta.glob<{ desktop: Record<string, string>; mobile: Record<string, string> }>(
  '../locales/*.json',
  { import: 'default' },
);

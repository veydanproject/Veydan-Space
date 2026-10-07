// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The light entry: the dictionary alone, no stores on the import path of i18n.
export { translations } from '$lib/ssh/i18n';

/** The other languages (core/i18n.ts fetches a language's file when it is first shown). */
export const locales = import.meta.glob<{ desktop: Record<string, string>; mobile: Record<string, string> }>(
  '../locales/*.json',
  { import: 'default' },
);

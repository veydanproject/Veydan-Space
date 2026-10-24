// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The words of «печатает…». Pure: the line passes the names, `t` and the
// language; the tests pass their own.

import { countKey, type Locale, type TranslationKey } from '$lib/core/i18n';

type Tr = (key: TranslationKey, vars?: Record<string, string>) => string;

/**
 * Nobody named: the peer of a direct chat, whose name is in the header
 * above. One or two: by name. More: the first two and how many besides.
 */
export function typingText(names: readonly string[], tr: Tr, loc: Locale): string {
  const [a, b] = names;
  if (!names.length) return tr('msg_typing');
  if (names.length === 1) return tr('msg_typing_1', { name: a });
  if (names.length === 2) return tr('msg_typing_2', { a, b });
  const n = names.length - 2;
  return tr(countKey('msg_typing_more', n, loc), { a, b, n: String(n) });
}

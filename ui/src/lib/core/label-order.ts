// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { get } from 'svelte/store';
import { locale } from '$lib/core/i18n';

const collators = new Map<string, Intl.Collator>();

/** Nav order for folders, tags, profiles and workspaces. Follows the app language, not the OS. */
export function compareLabel(a: string, b: string): number {
  const lang = get(locale);
  let collator = collators.get(lang);
  if (!collator) {
    collator = new Intl.Collator(lang, { sensitivity: 'base', numeric: true });
    collators.set(lang, collator);
  }
  return collator.compare(a, b);
}

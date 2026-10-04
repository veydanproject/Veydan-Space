// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The notes linked to a TOTP entry: the link is a `totp:<id>` reference on
// the note, kept by the notes module and reached through the catalog of
// entities. In a product without the notes there are none.

import { directory } from '$lib/core/directory';

/** Note ids that already store `totp:id`. */
export function notesLinkedToTotp(totpId: string): string[] {
  return directory.get('note')?.referring?.(`totp:${totpId}`) ?? [];
}

/** Write `totp:id` onto notes in `next` and drop it from notes no longer selected. */
export async function syncTotpNotes(totpId: string, next: string[], prev: string[]): Promise<void> {
  const link = directory.get('note')?.link;
  if (!link) return;
  const reference = `totp:${totpId}`;
  const want = new Set(next);
  const had = new Set(prev);
  for (const id of next) if (!had.has(id)) await link.add(id, reference);
  for (const id of prev) if (!want.has(id)) await link.remove(id, reference);
}

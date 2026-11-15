// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Which messages have words to copy, or to select in their own window
// (content/SelectTextDialog.svelte): a text, or the caption of a photo,
// a video or a file, while the message is there.

import type { MessengerMessage } from '../api';

/** Whether «Копировать» and «Выбрать текст» are offered for `m`. */
export function hasText(m: Pick<MessengerMessage, 'text' | 'deleted'>): boolean {
  return !!m.text && !m.deleted;
}

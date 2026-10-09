// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Which of my messages may be edited: a text, or the caption of a photo,
// a video or a file once it has left. An upload on its way is swapped for
// a message with another id, so an edit of it would point at nothing.

import type { MessengerMessage } from '../api';

type Editable = Pick<MessengerMessage, 'id' | 'direction' | 'status' | 'content_type' | 'deleted'>;

/** Whether the Edit item is offered for `m`; `canSend`: this chat takes words from me now. */
export function canEdit(m: Editable, canSend: boolean): boolean {
  if (m.direction !== 'out' || m.deleted || !canSend) return false;
  if (m.content_type === 'text') return true;
  return m.content_type === 'media' && !m.id.startsWith('local:') && m.status !== 'uploading' && m.status !== 'paused' && m.status !== 'failed';
}

/**
 * What an edit from the menu of `pressed` changes. An album's caption is
 * the text of one of its parts (the last one, as this app sends it), so
 * whichever part is pressed the caption is edited where it is.
 */
export function editTarget<M extends Pick<MessengerMessage, 'text'>>(pressed: M, album?: readonly M[]): M {
  if (!album?.length) return pressed;
  return album.findLast((x) => !!x.text?.trim()) ?? album[album.length - 1];
}

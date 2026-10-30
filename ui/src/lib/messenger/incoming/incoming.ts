// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What a link the system handed to the app shows (`veydan://…` clicked
// outside it; $lib/core/links). The runtime reads the link and says what it
// leads to; a link of a card is shown as the same card a link in a message
// gets, and everything else as a note with no button but «Close». A
// sign-in, a recovery and the linking of a device are refused by the
// runtime (`link_scanner_only`); should a card of theirs come anyway, it is
// refused here as well.

import type { LinkView } from '../api';
import type { InternalLinkText } from '../content/types';

export type NoteKey =
  | 'msg_inlink_scanner_title' | 'msg_inlink_scanner_text'
  | 'msg_inlink_long_title' | 'msg_inlink_long_text'
  | 'msg_card_invalid_title' | 'msg_card_invalid_text'
  | 'msg_card_unknown_title' | 'msg_card_unknown_text';

export type Shown =
  /** The card of the link, as under a message: it acts only on its button. */
  | { kind: 'card'; link: InternalLinkText }
  | { kind: 'note'; icon: string; title: NoteKey; text: NoteKey; params?: Record<string, string> };

const SCANNER_ONLY: Shown = { kind: 'note', icon: 'scan', title: 'msg_inlink_scanner_title', text: 'msg_inlink_scanner_text' };

export function shownOf(view: LinkView): Shown {
  switch (view.kind) {
    case 'invalid':
      if (view.code === 'link_scanner_only') return SCANNER_ONLY;
      if (view.code === 'link_too_long') return { kind: 'note', icon: 'alert-triangle', title: 'msg_inlink_long_title', text: 'msg_inlink_long_text' };
      return { kind: 'note', icon: 'alert-triangle', title: 'msg_card_invalid_title', text: 'msg_card_invalid_text' };
    case 'unknown':
      return { kind: 'note', icon: 'info', title: 'msg_card_unknown_title', text: 'msg_card_unknown_text', params: { type: view.link_type } };
    case 'web':
    case 'recover':
      return SCANNER_ONLY;
    default:
      return { kind: 'card', link: view.link as InternalLinkText };
  }
}

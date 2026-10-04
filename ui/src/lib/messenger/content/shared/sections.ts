// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Sections of what a chat has shared. The table has a row for every
// section the runtime knows: a new section without a row does not
// compile. Which messages fall into a section is decided by the runtime;
// `inSection` says the same for the demo and for tests.

import type { TranslationKey } from '$lib/core/i18n';
import type { MessengerMessage } from '../../api';
import type { SharedSection } from '../../generated/shared';
import { linksOf, tokenize } from '../tokenize';
import type { LinkSegment } from '../types';

interface SectionRow {
  icon: string;
  label: TranslationKey;
  empty: TranslationKey;
  /** Media kinds of the section; none for links. */
  kinds: readonly string[];
}

export const SECTIONS: { [K in SharedSection]: SectionRow } = {
  visual: { icon: 'image', label: 'msg_shared_visual', empty: 'msg_shared_empty_visual', kinds: ['image', 'video'] },
  files: { icon: 'file', label: 'msg_shared_files', empty: 'msg_shared_empty_files', kinds: ['file', 'audio'] },
  links: { icon: 'link', label: 'msg_shared_links', empty: 'msg_shared_empty_links', kinds: [] },
  voice: { icon: 'mic', label: 'msg_shared_voice', empty: 'msg_shared_empty_voice', kinds: ['voice', 'circle'] },
};

/** In the order the panel lists them. */
export const SECTION_ORDER: SharedSection[] = ['visual', 'files', 'links', 'voice'];

const HAS_LINK = /https?:\/\/|veydan:\/\/|npub1/i;

/** Is the message in the section? A caption with a link puts a picture in two. */
export function inSection(m: MessengerMessage, section: SharedSection): boolean {
  if (m.deleted) return false;
  if (section === 'links') return (m.content_type === 'text' || m.content_type === 'media') && !!m.text && HAS_LINK.test(m.text);
  const kind = (m.media as { kind?: unknown } | null)?.kind;
  return m.content_type === 'media' && SECTIONS[section].kinds.includes(kind as string);
}

export interface LinkItem {
  message: MessengerMessage;
  link: LinkSegment;
}

/** Links of the messages, each once per message, in the order of the messages. */
export function linkItems(messages: MessengerMessage[]): LinkItem[] {
  const out: LinkItem[] = [];
  for (const message of messages) {
    if (!message.text) continue;
    for (const link of linksOf(tokenize(message.text))) out.push({ message, link });
  }
  return out;
}

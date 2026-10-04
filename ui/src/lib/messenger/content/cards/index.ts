// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Which card draws which kind of link. The table has a row for every
// kind the runtime may answer with: a new kind without a card does not
// compile.

import type { Component } from 'svelte';
import type { LinkView } from '../../api';
import BridgeCard from './BridgeCard.svelte';
import ContactCard from './ContactCard.svelte';
import GroupCard from './GroupCard.svelte';
import UnknownCard from './UnknownCard.svelte';

export type LinkKind = LinkView['kind'];

type CardOf<K extends LinkKind> = Component<{ view: Extract<LinkView, { kind: K }>; text: string }>;

export const CARDS: { [K in LinkKind]: CardOf<K> } = {
  group: GroupCard,
  contact: ContactCard,
  vlink: BridgeCard,
  unknown: UnknownCard,
  invalid: UnknownCard,
};

/** What the text of a message shows in place of a link inside. */
export const ICONS: Record<LinkKind, string> = {
  group: 'users',
  contact: 'user',
  vlink: 'shield',
  unknown: 'info',
  invalid: 'alert-triangle',
};

/** The card of a view, to be given that view. */
export function cardOf(view: LinkView): Component<{ view: LinkView; text: string }> {
  return CARDS[view.kind] as Component<{ view: LinkView; text: string }>;
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The cards of the bot of Veydan Cloud, between the messenger and the
// module that knows them (internal/messenger-wire.md §3; platform-spec
// 6.3: a module sees of the messenger its seams alone). The messenger
// keeps the chat and says which message is a card; what a card says and
// does is the module's, given here once at its start. A product without
// that module gives nothing: every card is then the neutral «Сообщение
// Veydan Cloud».

import type { Component } from 'svelte';
import type { TranslationKey } from '$lib/core/i18n';
import type { BotCard } from '../generated/bot';

export type { BotCard };

/** What the component of a card is given. */
export interface BotCardProps {
  /** The card as the runtime read it: its type and the typed values of the table, nothing else. */
  card: BotCard;
  /** The id of the message: the id of the card everywhere else (`BotCards::dismiss`). */
  id: string;
  /** When the bot wrote it, unix seconds. */
  at: number;
}

/** What a module gives for the cards. */
export interface BotCardHost {
  /** The card in the chat: fixed words by its type, at most one button. */
  card: Component<BotCardProps>;
  /** One short line of fixed words for the list of chats and a notice; null: the neutral one. */
  line: (card: BotCard) => TranslationKey | null;
}

class BotCardSeam {
  // Raw: a deep `$state` would hand back a proxy of the host, which is not
  // the host a module gave, and giving the cards back compares the two.
  host = $state.raw<BotCardHost | null>(null);
}

export const botCards = new BotCardSeam();

/** The module takes the cards; returns the way to give them back. */
export function provideBotCards(host: BotCardHost): () => void {
  botCards.host = host;
  return () => {
    if (botCards.host === host) botCards.host = null;
  };
}

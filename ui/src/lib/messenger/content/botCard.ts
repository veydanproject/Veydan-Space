// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A card of the bot of Veydan Cloud as the messenger shows it where no
// component is drawn: one line of fixed words in the list of chats and in
// a notice. The words by type are of the module that took the cards
// (`seams/cards`); without it, and for a type nobody knows, the neutral
// line. Nothing of a card's fields is ever part of the line.

import type { TranslationKey } from '$lib/core/i18n';
import type { BotCard, BotCardHost } from '../seams/cards.svelte';

/** «Сообщение Veydan Cloud»: the card of an unknown type, and every card where no module knows them. */
export const NEUTRAL: TranslationKey = 'msg_bot_card_title';

/** The key of the line of `card`. */
export function botCardLineKey(card: BotCard, host: BotCardHost | null): TranslationKey {
  if (card.t === 'unknown' || !host) return NEUTRAL;
  return host.line(card) ?? NEUTRAL;
}

/** Whether the module draws `card`; otherwise the messenger draws the neutral one. */
export function drawnByHost(card: BotCard, host: BotCardHost | null): host is BotCardHost {
  return card.t !== 'unknown' && host !== null;
}

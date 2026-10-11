// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The cards of the bot of Veydan Cloud are this module's to word and to act
// on (cloud-recovery.md §10): it takes them from the messenger through the
// messenger's seam when it starts and gives them back when it stops. While
// it holds none (the module is off, or absent from the product) the
// messenger shows every card as «Сообщение Veydan Cloud».

import { provideBotCards } from '$lib/messenger/seams/cards.svelte';
import BotCard from '$lib/cloud/BotCard.svelte';
import { cardLine } from '$lib/cloud/botCard';

let drop: (() => void) | null = null;

/** `ModuleDef.start`; idempotent. */
export async function takeBotCards(): Promise<void> {
  drop ??= provideBotCards({ card: BotCard, line: cardLine });
}

/** `ModuleDef.stop`. */
export function dropBotCards(): void {
  drop?.();
  drop = null;
}

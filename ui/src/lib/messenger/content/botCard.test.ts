// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A card of the bot of Veydan Cloud where the messenger words it itself:
// the neutral «Сообщение Veydan Cloud» for a type nobody knows and in a
// product without the module that knows the cards; the module's line
// otherwise. Only what the runtime calls a card is drawn as one.

import { describe, expect, it } from 'vitest';
import type { Component } from 'svelte';
import type { TranslationKey } from '$lib/core/i18n';
import { messengerTranslations } from '../i18n';
import { botCards, provideBotCards, type BotCard, type BotCardHost, type BotCardProps } from '../seams/cards.svelte';
import { botCardLineKey, drawnByHost, NEUTRAL } from './botCard';
import neutralSource from './BotCardMessage.svelte?raw';
import seamSource from '../seams/cards.svelte.ts?raw';
import bubbleSource from '../dm/MessageBubble.svelte?raw';
import windowSource from '../dm/ChatWindow.svelte?raw';
import callButtonSource from '../calls/CallButton.svelte?raw';
import callLineSource from '../calls/CallLine.svelte?raw';

const card = (() => {}) as unknown as Component<BotCardProps>;
const host: BotCardHost = {
  card,
  line: (c) => (c.t === 'recovery.requested' ? ('cloud_card_line_requested' as TranslationKey) : null),
};
const requested: BotCard = { t: 'recovery.requested', request_id: '9c', created_at: 1, expires_at: 2 };

describe('the line of a card of the bot', () => {
  it('is the neutral one without a module that knows the cards', () => {
    expect(NEUTRAL).toBe('msg_bot_card_title');
    expect(botCardLineKey(requested, null)).toBe(NEUTRAL);
    expect(messengerTranslations.en.msg_bot_card_title).toBe('Message from Veydan Cloud');
    expect(messengerTranslations.ru.msg_bot_card_title).toBe('Сообщение Veydan Cloud');
  });

  it('is the module\'s by the type, and neutral for what the module does not word', () => {
    expect(botCardLineKey(requested, host)).toBe('cloud_card_line_requested');
    expect(botCardLineKey({ t: 'recovery.restored', at: 1 }, host)).toBe(NEUTRAL);
  });

  it('of an unknown type is neutral whoever took the cards', () => {
    expect(botCardLineKey({ t: 'unknown' }, host)).toBe(NEUTRAL);
    expect(drawnByHost({ t: 'unknown' }, host)).toBe(false);
    expect(drawnByHost(requested, host)).toBe(true);
    expect(drawnByHost(requested, null)).toBe(false);
  });
});

describe('the seam of the cards', () => {
  it('holds one module, and gives the cards back when it stops', () => {
    expect(botCards.host).toBeNull();
    const drop = provideBotCards(host);
    expect(botCards.host).toBe(host);
    const other: BotCardHost = { card, line: () => null };
    const dropOther = provideBotCards(other);
    // The one that stopped does not take away what another gave since.
    drop();
    expect(botCards.host).toBe(other);
    dropOther();
    expect(botCards.host).toBeNull();
  });

  it('keeps the host as it was given, not a proxy of it', () => {
    // Tests compile `$state` for the server, a plain field; in the browser a
    // deep one wraps the host, `botCards.host === host` is never true and
    // the cards are never given back. Only the source tells the two apart.
    expect(seamSource).toContain('host = $state.raw<BotCardHost | null>(null);');
    expect(seamSource).not.toMatch(/\$state<BotCardHost/);
  });
});

describe('the card in the chat', () => {
  it('is drawn only for what the runtime calls a card, never by the type a message names', () => {
    expect(bubbleSource).toContain('{:else if m.bot_card}');
    expect(bubbleSource).not.toContain("content_type === 'bot_card'");
    expect(bubbleSource).toContain('<BotCardMessage card={m.bot_card} id={m.id} at={m.created_at} />');
  });

  it('stands in a chat that offers nothing the runtime gives the bot none of', () => {
    // The chat is a full one, and still: no reaction (the runtime answers «wait until it accepts»),
    // no call («calls go to contacts only»), no «add to contacts».
    expect(windowSource).toMatch(/const reactable = [^;]*chat\.can_send && !chat\.bot &&/);
    expect(windowSource).toMatch(/chat\.mode !== "request_received" && !chat\.bot\) list\.push\(\{ label: \$t\("msg_rel_cta_add"\)/);
    expect(callButtonSource).toMatch(/const callable = [^;]*chat\.mode === 'full_chat' && !chat\.bot/);
    expect(callLineSource).toMatch(/const callable = [^;]*chat\.mode === 'full_chat' && !chat\.bot/);
  });

  it('without a module is a title and nothing of the card', () => {
    expect(neutralSource).toContain('{$t(NEUTRAL)}');
    expect(neutralSource).not.toContain('{@html');
    expect(neutralSource).not.toMatch(/<a[\s>]/);
    expect(neutralSource).not.toContain('<button');
    expect(neutralSource).not.toContain('MessageContent');
  });
});

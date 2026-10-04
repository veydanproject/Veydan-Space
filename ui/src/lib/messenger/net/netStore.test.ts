// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A card of a bridge link shows the failure of its own call only: a link
// refused in the network panel must not appear on every card in a chat.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import cardSource from '../content/cards/BridgeCard.svelte?raw';
import offerSource from './BridgeOffer.svelte?raw';
import panelSource from './NetPanel.svelte?raw';

const net = vi.hoisted(() => ({
  addBridge: vi.fn(async (link: string): Promise<void> => {
    if (!link.startsWith('veydan://vlink/')) throw new Error('invalid input: net_bridge_link_type');
  }),
}));
vi.mock('../api', () => ({
  messengerApi: { net },
  messengerError: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));

const { netStore } = await import('./netStore.svelte');

const GOOD = 'veydan://vlink/abc?a=203.0.113.7:443';
const OLD = 'veydan://bridge/abc?a=203.0.113.7:443';

describe('adding a bridge from a card', () => {
  beforeEach(() => netStore.reset());

  it('answers with nothing when the bridge was added', async () => {
    expect(await netStore.addBridgeFromCard(GOOD)).toBe('');
    expect(netStore.error).toBe('');
    expect(netStore.busy).toBe(false);
  });

  it('answers with the failure of its own call', async () => {
    expect(await netStore.addBridgeFromCard(OLD)).toBe('invalid input: net_bridge_link_type');
  });

  it('is not given a failure of the network panel', async () => {
    expect(await netStore.addBridge(OLD)).toBe(false);
    expect(netStore.error).toContain('net_bridge_link_type');
    // The panel's failure stays the panel's; the card's own call succeeds clean.
    expect(await netStore.addBridgeFromCard(GOOD)).toBe('');
  });
});

describe('the screens that show a failure of the network', () => {
  const screens = {
    card: cardSource,
    offer: offerSource,
    panel: panelSource,
  };

  it('never print the runtime wording as it came', () => {
    for (const [name, source] of Object.entries(screens)) {
      expect(source, name).not.toMatch(/\{\s*netStore\.error\s*\}/);
      expect(source, name).toContain('netErrorText(');
    }
  });

  it('a card does not read the shared failure at all', () => {
    expect(screens.card).not.toContain('netStore.error');
    expect(screens.card).toContain('addBridgeFromCard(');
  });
});

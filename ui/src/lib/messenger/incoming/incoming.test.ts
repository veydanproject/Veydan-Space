// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { LinkView } from '../api';
import cardSource from '../content/LinkCard.svelte?raw';
import dialogSource from './IncomingLink.svelte?raw';
import { shownOf } from './incoming';

const server = {
  link: 'veydan://server/abc', id: 'ab'.repeat(8), name: 'S1', relay: 'wss://r.example', media: null, call_node: null,
  push_url: null, push: 'off', known: false, is_veydan: false,
};

describe('a link from the system', () => {
  it('opens the card of its kind, by the link as the runtime writes it', () => {
    const cards: LinkView[] = [
      { kind: 'contact', link: 'veydan://contact/npub1a', pubkey: 'a', npub: 'npub1a', name: '', picture: null, nip05: null, is_me: false, is_contact: false, blocked: false, invite: null, server: null },
      { kind: 'group', link: 'veydan://group/g', group_id: 'g', group_kind: 'public', name: 'G', relay: 'wss://r', owner: 'o', picture: null, members: null, membership: null, server: null, server_known: true },
      { kind: 'channel', link: 'veydan://channel/c', community_id: 'c', relay: 'wss://r', owner: 'o', has_secret: true, invited: false },
      { kind: 'server', ...server },
      { kind: 'vlink', link: 'veydan://vlink/v', id: 'v', addr: '1.2.3.4:443', added: false },
      { kind: 'call_node', link: 'veydan://call-node/n', id: 'n', addr: '1.2.3.4:443', has_token: true, added: false },
    ];
    for (const view of cards) {
      expect(shownOf(view)).toEqual({ kind: 'card', link: (view as { link: string }).link });
    }
  });

  it('is refused with the hint of the scanner for a sign-in, a recovery and a device link', () => {
    const scanner = { kind: 'note', icon: 'scan', title: 'msg_inlink_scanner_title', text: 'msg_inlink_scanner_text' };
    expect(shownOf({ kind: 'invalid', code: 'link_scanner_only' })).toEqual(scanner);
    // Refused by the runtime already; refused here too should one come.
    expect(shownOf({ kind: 'web', link: 'veydan://web/0123456789abcdef', id: '0123456789abcdef' })).toEqual(scanner);
    expect(shownOf({ kind: 'recover', link: 'veydan://recover/a?c=X', npub: 'a', code: 'X', name: '' })).toEqual(scanner);
  });

  it('says what is wrong with a long, a broken and an unknown link, with nothing to press', () => {
    expect(shownOf({ kind: 'invalid', code: 'link_too_long' })).toMatchObject({ kind: 'note', title: 'msg_inlink_long_title' });
    expect(shownOf({ kind: 'invalid', code: 'link_bad_type' })).toMatchObject({ kind: 'note', title: 'msg_card_invalid_title' });
    expect(shownOf({ kind: 'unknown', link_type: 'future' })).toEqual({
      kind: 'note', icon: 'info', title: 'msg_card_unknown_title', text: 'msg_card_unknown_text', params: { type: 'future' },
    });
  });

  it('makes a new card for every link, so nothing of the one before carries over', () => {
    expect(dialogSource).toMatch(/\{#key shown\}[\s\S]*<LinkCard[\s\S]*\{\/key\}/);
  });

  it('asks the relay of the link nothing before a press, not even the name of a channel', () => {
    expect(dialogSource).toMatch(/<LinkCard [^>]*preview=\{false\}/);
    expect(cardSource).toMatch(/if \(!preview \|\|[^)]*\) return;\s*let alive[\s\S]*host\.linkTitle\(/);
  });
});

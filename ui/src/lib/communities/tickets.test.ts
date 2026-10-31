// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { ChannelDetail } from './api';
import { showsTickets, ticketsToAsk } from './tickets';

function channel(role: string, join: string, kept_here = false): ChannelDetail {
  return {
    channel: {
      id: 'c', chat_id: 'channel:c', name: 'C', about: '', owner: 'o', join, membership: 'joined', my_role: role, members: 3,
      relays: [], storage: [], link_epoch: 1, kept_here, needs_update: false, unread: 0, pinned: false, muted: false, archived: false, created_at: 0, updated_at: 0, last_at: 0,
    },
    history: 'open', post_mode: 'authors', comments: 'on', premod: 'off', pinned: [], disbanded: false, has_key: true, waiting: 0, waiting_keeper: 0, keeper_refused: false, staff: [],
  } as ChannelDetail;
}

describe('the tickets of a moderator (communities-wire §17)', () => {
  it('are shown to a moderator of a request or invite channel kept elsewhere', () => {
    expect(showsTickets(channel('moderator', 'request'))).toBe(true);
    expect(showsTickets(channel('admin', 'invite'))).toBe(true);
    expect(showsTickets(channel('moderator', 'open'))).toBe(false);
    expect(showsTickets(channel('author', 'request'))).toBe(false);
    expect(showsTickets(channel('owner', 'request', true))).toBe(false);
  });

  it('ask for as many as the hand lacks', () => {
    expect(ticketsToAsk({ held: 12, max: 50 })).toBe(38);
    expect(ticketsToAsk({ held: 50, max: 50 })).toBe(0);
    expect(ticketsToAsk(null)).toBe(0);
  });
});

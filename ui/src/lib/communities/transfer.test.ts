// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { ChannelDetail, TransferView } from './api';
import { canTransferTo, myOffer, offerToMe } from './transfer';

const ME = 'aa'.repeat(32);
function channel(role: string, transfer?: TransferView): ChannelDetail {
  return {
    channel: {
      id: 'c', chat_id: 'channel:c', name: 'C', about: '', owner: role === 'owner' ? ME : 'o', join: 'open', membership: 'joined', my_role: role, members: 3,
      relays: [], storage: [], link_epoch: 1, kept_here: false, needs_update: false, unread: 0, pinned: false, muted: false, archived: false, created_at: 0, updated_at: 0, last_at: 0,
    },
    history: 'open', post_mode: 'authors', comments: 'on', premod: 'off', pinned: [], disbanded: false, has_key: true, waiting: 0, waiting_keeper: 0, keeper_refused: false, staff: [], transfer,
  };
}
const t = (direction: 'in' | 'out', status: 'offered' | 'declined'): TransferView => ({ direction, peer: 'bb'.repeat(32), status, created_at: 1 });

describe('a channel handed to a new owner (communities-wire §16)', () => {
  it('shows the offer to the one it is made to until he answers', () => {
    expect(offerToMe(channel('moderator', t('in', 'offered')))?.peer).toBe('bb'.repeat(32));
    expect(offerToMe(channel('moderator', t('in', 'declined')))).toBeNull();
    expect(offerToMe(channel('owner', t('out', 'offered')))).toBeNull();
    expect(offerToMe(channel('moderator'))).toBeNull();
  });

  it('shows the owner his offer, waiting or declined', () => {
    expect(myOffer(channel('owner', t('out', 'offered')))?.status).toBe('offered');
    expect(myOffer(channel('owner', t('out', 'declined')))?.status).toBe('declined');
    expect(myOffer(channel('owner'))).toBeNull();
  });

  it('offers «Передать канал» to the owner alone, on somebody else, one offer at a time', () => {
    const other = 'cc'.repeat(32);
    expect(canTransferTo(channel('owner'), other, ME)).toBe(true);
    expect(canTransferTo(channel('owner'), ME, ME)).toBe(false);
    expect(canTransferTo(channel('admin'), other, ME)).toBe(false);
    expect(canTransferTo(channel('owner', t('out', 'offered')), other, ME)).toBe(false);
    expect(canTransferTo(channel('owner', t('out', 'declined')), other, ME)).toBe(true);
  });
});

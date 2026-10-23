// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A channel handed to a new owner (communities-wire §16): what the feed of
// the one offered it shows («Вам передают канал»), what the owner's page
// shows of his offer, and to whom «Передать канал» is offered.

import type { ChannelDetail, TransferView } from './api';
import { isOwner } from './permissions';

/** An offer of the owner to me, not answered yet. */
export const offerToMe = (d: ChannelDetail | null | undefined): TransferView | null =>
  d?.transfer?.direction === 'in' && d.transfer.status === 'offered' ? d.transfer : null;

/** My offer of the channel, while it waits or after it was declined. */
export const myOffer = (d: ChannelDetail | null | undefined): TransferView | null => (d?.transfer?.direction === 'out' ? d.transfer : null);

/** «Передать канал» on a person of the team: the owner's, to somebody else, one offer at a time. */
export const canTransferTo = (d: ChannelDetail | null | undefined, who: string, me: string | null): boolean =>
  isOwner(d) && who !== me && myOffer(d)?.status !== 'offered';

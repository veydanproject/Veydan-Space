// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The tickets of a moderator (communities-wire §17): the one line of the
// moderation «Билеты: 12 / запросить». A moderator holds them to let a
// newcomer read at once while the keeper is away; an open channel lets
// people in by its link, and the device of the keeper needs none.

import type { ChannelDetail, TicketsView } from './api';
import { isModerator } from './permissions';

/** The line is shown to a moderator of a `request` or `invite` channel kept on another device. */
export const showsTickets = (d: ChannelDetail | null | undefined): d is ChannelDetail =>
  isModerator(d) && d.channel.join !== 'open' && !d.channel.kept_here;

/** How many to ask for: up to the most one holds; none when the hand is full. */
export const ticketsToAsk = (t: TicketsView | null | undefined): number => (t ? Math.max(0, t.max - t.held) : 0);

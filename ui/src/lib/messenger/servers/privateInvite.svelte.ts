// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// «Пригласить в частный чат» (PrivateInviteDialog): the one place a private
// chat on a server of mine is offered from — the menu of a chat with a
// person, their page, a server in the settings, «Новый чат». With a person
// the invitation goes to them (my card with the server, in our home chat)
// and the private chat opens here; with or without one, my link of that
// chat can be copied (it carries the server and an invitation: whoever
// opens it, someone I never talked to too, adds the server and is let
// straight in) or sent into any chat.

import { chatStore } from '../chats/chatStore.svelte';
import { openChat } from '../content/actions';
import { messengerApi, type MessengerChat } from '../api';
import { messengerStore } from '../store.svelte';

export interface PrivateInviteAsk {
  /** The person to invite (hex); none: a link for anyone. */
  peer?: string | null;
  /** The server to start with; the first of mine when none. */
  server?: string | null;
}

class PrivateInviteState {
  current = $state<PrivateInviteAsk | null>(null);

  show(ask: PrivateInviteAsk = {}): void {
    this.current = { peer: ask.peer ?? null, server: ask.server ?? null };
  }

  close(): void {
    this.current = null;
  }
}

export const privateInvite = new PrivateInviteState();

/** My link of the private chat on `server`, with its invitation in it. */
export async function privateLink(server: string): Promise<string> {
  const me = messengerStore.identity?.pubkey;
  if (!me) throw new Error('no_identity');
  return messengerApi.links.contact(me, server);
}

/**
 * Invite `peer` to the private chat on `server`: my card with the server
 * goes into our home chat (the runtime approves them there on my side),
 * and the private chat opens here.
 */
export async function invitePeer(peer: string, server: string): Promise<void> {
  const home = chatStore.chats.find((c) => c.id === `dm:${peer}`) ?? ({ id: `dm:${peer}` } as MessengerChat);
  await chatStore.sendCard(home, null, false, server);
  const chat = await messengerApi.servers.openPrivate(peer, server);
  await openChat(chat.id);
}

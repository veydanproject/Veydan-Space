// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the cards do. Each action takes the view the runtime gave, never
// the text of a message: what was shown is what is acted on.

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { messengerApi, messengerError, type CardView, type LinkView, type ServerPreview } from '../api';
import { acceptServer } from '../servers/serverAccept.svelte';
import { copied as copiedToast, toast } from '$lib/core/ui/toast.svelte';
import { chatStore } from '../chats/chatStore.svelte';
import { groupStore } from '../groups/groupStore.svelte';
import { messengerStore } from '../store.svelte';
import { linkStore } from './linkStore.svelte';
import type { ExternalUrl } from './types';

type GroupView = Extract<LinkView, { kind: 'group' }>;
type ContactView = Extract<LinkView, { kind: 'contact' }>;

/** How a chat is shown once it is open. A phone goes to its page; a desk has it in place. */
let show: (chatId: string) => void | Promise<void> = () => {};

export function onChatOpened(fn: (chatId: string) => void | Promise<void>): () => void {
  const before = show;
  show = fn;
  return () => { if (show === fn) show = before; };
}

/** Opens a chat and shows it, wherever the user was in the messenger. */
export async function openChat(chatId: string) {
  await chatStore.loadChats();
  await chatStore.open(chatId);
  await show(chatId);
}

export const linkActions = {
  openExternal: (url: ExternalUrl) => messengerApi.openUrl(url),

  /** A tap on a link: to the system's browser; a failure says so in a toast, with «Повторить». */
  async follow(url: ExternalUrl, retryOf?: number): Promise<void> {
    try {
      await messengerApi.openUrl(url);
    } catch (e) {
      const id: number = toast.error((get(t) as (k: string) => string)('msg_link_open_failed'), {
        sub: messengerError(e),
        retryOf,
        onAction: () => void linkActions.follow(url, id),
      });
    }
  },

  /**
   * Come into a public group, or ask a private one. A group on a server of
   * its own (the link carries its card) is joined there, after «Добавить
   * сервер?» said yes; `null` when it said no. Returns what I am to the group now.
   */
  async joinGroup(view: GroupView, note = '') {
    if (view.server && !view.server_known) {
      if (!(await acceptServer(view.server))) return null;
    }
    const g = await groupStore.openLink(view.link, note.trim());
    linkStore.refresh();
    if (g.membership === 'joined' || g.membership === 'joining') await openChat(g.chat_id);
    else await chatStore.loadChats();
    return g.membership;
  },

  openGroup: (view: GroupView) => openChat(`group:${view.group_id}`),

  async write(view: ContactView) {
    if (view.server) {
      await linkActions.writePrivate(view);
      return;
    }
    const chat = await chatStore.openPeer(view.pubkey);
    await show(chat.id);
  },

  /**
   * The private chat with a person on the server their link names: the
   * server is added after «Добавить сервер?» said yes, then the chat opens
   * there, approved on my side and told at once (with the link's invitation).
   * `false` when the question was answered no.
   */
  async writePrivate(view: ContactView): Promise<boolean> {
    if (!view.server) return false;
    return openPrivateOn(view.server, view.pubkey, view.invite);
  },

  async addContact(view: ContactView) {
    await messengerStore.addContact(view.pubkey);
    linkStore.refresh();
  },

  copy: (text: string) => navigator.clipboard.writeText(text).then(() => { copiedToast(); }),
};

/** What a contact card in a chat does. It acts on the message: the runtime reads the card it checked. */
export const cardActions = {
  /** Adds the person; a phone in the sender's own card is kept with the contact. */
  async accept(messageId: string) {
    const m = await chatStore.acceptCard(messageId);
    await messengerStore.refreshContacts();
    linkStore.refresh();
    // The runtime says `contact_private.updated` too; asking here keeps a panel that is open right.
    if (m.card?.phone) messengerStore.loadContactPhone(m.card.pubkey).catch(() => {});
    return m;
  },

  async write(pubkey: string) {
    const chat = await chatStore.openPeer(pubkey);
    await show(chat.id);
  },

  /**
   * The private chat the card offers, on the server of its sender: what the
   * server is is asked of the runtime, «Добавить сервер?» asks, then the chat
   * opens there and the sender's side, which approved me by sending the card,
   * opens too. `false` when the question was answered no.
   */
  async openPrivate(card: CardView): Promise<boolean> {
    if (!card.server) return false;
    const server = await messengerApi.servers.preview(card.server);
    return openPrivateOn(server, card.pubkey, null);
  },
};

/** The server mine (asked first), then the private chat with `pubkey` there, shown. */
async function openPrivateOn(server: ServerPreview, pubkey: string, invite: string | null): Promise<boolean> {
  const id = await acceptServer(server);
  if (!id) return false;
  const chat = await messengerApi.servers.openPrivate(pubkey, id, invite);
  linkStore.refresh();
  await openChat(chat.id);
  return true;
}

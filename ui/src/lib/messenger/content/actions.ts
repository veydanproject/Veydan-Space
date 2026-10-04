// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the cards do. Each action takes the view the runtime gave, never
// the text of a message: what was shown is what is acted on.

import { messengerApi, type LinkView } from '../api';
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

  /** Come into a public group, or ask a private one. Returns what I am to the group now. */
  async joinGroup(view: GroupView, note = '') {
    const g = await groupStore.openLink(view.link, note.trim());
    linkStore.refresh();
    if (g.membership === 'joined' || g.membership === 'joining') await openChat(g.chat_id);
    else await chatStore.loadChats();
    return g.membership;
  },

  openGroup: (view: GroupView) => openChat(`group:${view.group_id}`),

  async write(view: ContactView) {
    const chat = await chatStore.openPeer(view.pubkey);
    await show(chat.id);
  },

  async addContact(view: ContactView) {
    await messengerStore.addContact(view.pubkey);
    linkStore.refresh();
  },

  copy: (text: string) => navigator.clipboard.writeText(text),
};

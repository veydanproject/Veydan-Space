// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What another app shared into Veydan, while the messenger holds it: the
// chooser of a chat shows `pending`; a chosen chat opens with the share and
// its ChatWindow takes it (`takeFor`) once its composer may attach; several
// chosen chats get it at once from here. The messenger is a reader of
// `core/shares` (`shares` of the module, entry/common.ts): the shell keeps
// the share until the runtime runs and the app is unlocked. The files are
// copies in the app's cache: whoever takes them tells the shell when they
// may go (`messengerApi.shares.forget`).

import type { IncomingShare, ShareTarget } from '$lib/core/module';
import { messengerApi, type MessengerChat } from '../api';
import { chatStore } from '../chats/chatStore.svelte';
import { chatAddress } from '../chats/address';
import { drafts, draftsLoaded } from '../shared/drafts.svelte';
import { goto } from '$app/navigation';
import { chatSections } from '../seams/sections.svelte';
import { onConversation, openHref, shouldFollow } from '../mobile/routes';
import { captionOf, directTarget, draftOf } from './shareIn';

/** What one chat got of a share sent to several at once. */
export interface ShareOutcome {
  done: string[];
  /** The chats that refused, with why. */
  short: { target: string; error: unknown }[];
}

class ShareInStore {
  /** The share the chooser shows; null while none. */
  pending = $state<IncomingShare | null>(null);
  /** What the other modules offer beside the chats («Новая заметка»). */
  others = $state.raw<ShareTarget[]>([]);
  /** The share a chosen chat opens with, until that chat's window takes it. */
  private forChat = $state.raw<{ chatId: string; share: IncomingShare } | null>(null);

  /** The core's inbox hands a share over: straight to the chat the system's sheet chose, else the chooser. */
  async offer(share: IncomingShare, others: ShareTarget[]) {
    this.others = others;
    await chatStore.loadChats().catch(() => {});
    const direct = directTarget(share, chatStore.chats);
    if (direct) {
      await this.toChat(share, direct.id);
      return;
    }
    this.pending = share;
  }

  /** The chooser is closed without a choice: the share is given up, its files go. */
  close() {
    const share = this.pending;
    this.pending = null;
    if (share) void messengerApi.shares.forget(share.id);
  }

  /** Another module takes the share (its `alt`): the chooser closes, the module forgets the files itself. */
  handOver() {
    this.pending = null;
  }

  /**
   * One chat chosen: it opens with the share. Its text goes into the
   * composer as the draft of that chat (one typed there before stays as it
   * is); its files are attached by the chat's window (`takeFor`), which
   * forgets the share then. A share of text alone is done here.
   */
  async toChat(share: IncomingShare, chatId: string) {
    this.pending = null;
    const text = draftOf(share);
    if (text && !drafts.get(chatId)) {
      drafts.set(chatId, text);
      // An open composer of this very chat takes the draft as one that came in.
      draftsLoaded.n++;
    }
    if (share.files.length || share.refused.length) this.forChat = { chatId, share };
    else void messengerApi.shares.forget(share.id);
    // The chat's page, from any screen (the sheet is shown over the whole
    // app): as a tapped notification goes, over another chat in its place.
    const route = openHref(chatId, !!chatSections.ofChat(chatId));
    if (shouldFollow(route, location)) await goto(route, { replaceState: onConversation(location.pathname) });
  }

  /** Whether a share waits for the window of `chatId` (read reactively). */
  waitsFor(chatId: string): boolean {
    return this.forChat?.chatId === chatId;
  }

  /** The share that waits for `chatId`, taken by its window: nothing until another is given. */
  takeFor(chatId: string): IncomingShare | null {
    if (this.forChat?.chatId !== chatId) return null;
    const share = this.forChat.share;
    this.forChat = null;
    return share;
  }

  /**
   * Several chats chosen: every file goes to each of them as one album, the
   * text and the comment as the caption of its last file (as a text alone
   * when there are no files). Each chat's copy is imported anew from the
   * share: the import makes a copy of its own. The files go once all chats
   * are done, however they ended.
   */
  async sendToMany(share: IncomingShare, targets: MessengerChat[], comment: string): Promise<ShareOutcome> {
    this.pending = null;
    const out: ShareOutcome = { done: [], short: [] };
    try {
      for (const chat of targets) {
        try {
          await this.sendTo(share, chat, comment);
          out.done.push(chat.id);
        } catch (error) {
          out.short.push({ target: chat.id, error });
        }
      }
    } finally {
      void messengerApi.shares.forget(share.id);
    }
    chatStore.scheduleChatsRefresh();
    return out;
  }

  private async sendTo(share: IncomingShare, chat: MessengerChat, comment: string) {
    const to = chatAddress(chat);
    if (!to) throw new Error('no chat');
    const caption = captionOf(share, comment);
    if (!share.files.length) {
      if (caption) await messengerApi.dm.sendText(to, caption);
      return;
    }
    const batch = share.files.length > 1 ? crypto.randomUUID() : undefined;
    for (const [i, f] of share.files.entries()) {
      const picked = await messengerApi.media.importPicked(f.path);
      const last = i === share.files.length - 1;
      await messengerApi.media.sendFile(to, picked.path, last ? caption : undefined, batch ? { id: batch, pos: i } : undefined);
    }
  }
}

export const shareStore = new ShareInStore();

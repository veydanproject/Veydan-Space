// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A call from the journal (the section «Звонки»): back to whom a call of
// it was with, or to a chat picked for a new call. A call between two asks
// first, as the button of the chat's header does (CallConfirm, the host is
// Dialer.svelte); a group's call joins the one that is on in the group, or
// starts one. A refusal is a toast: the journal has no line beside the
// button to say it.

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { toast } from '$lib/core/ui/toast.svelte';
import type { CallMedia, MessengerChat } from '../api';
import { chatAddress } from '../chats/address';
import { messengerStore } from '../store.svelte';
import { callStore } from './callStore.svelte';
import { groupCallStore } from './groupCallStore.svelte';
import { openGroupRoom } from './room';
import { callErrorText } from './words';

/** Whom a call goes to: a chat between two (its peer), or a group. */
export interface DialTarget {
  kind: 'dm' | 'group';
  chatId: string;
  /** The peer's key (hex) between two, the group's id for a group. */
  peer: string;
  /**
   * What the call is started with between two: the peer, with the server
   * of a private chat (`<hex>@<server>`), so the call goes on that server.
   */
  address: string;
  name: string;
  picture: string | null;
}

export function targetOf(chat: MessengerChat): DialTarget {
  const group = chat.kind === 'group';
  const peer = group ? chat.id.slice('group:'.length) : (chat.peer_pubkey ?? '');
  return {
    kind: group ? 'group' : 'dm',
    chatId: chat.id,
    peer,
    address: group ? peer : (chatAddress(chat) ?? ''),
    name: chat.title,
    picture: chat.picture ?? null,
  };
}

/**
 * Whether the chat can be called at all: a contact in a mutual chat (never
 * the bot), or a group I am in. Whether it can be called now is `dialBlock`.
 */
export function dialable(chat: MessengerChat | null | undefined): chat is MessengerChat {
  if (!chat) return false;
  if (chat.kind === 'group') return chat.can_send;
  return chat.mode === 'full_chat' && !chat.bot && !!chat.peer_pubkey;
}

/** Why no call can be made now ('' when one can): no engine, a call under way, the session locked. */
export function dialBlock(): string {
  const tr = get(t);
  if (callStore.loaded && !callStore.available) return tr('msg_call_unavailable');
  if (callStore.call || groupCallStore.call) return tr('msg_call_err_busy');
  if (!messengerStore.status?.runtime?.session_active) return tr('msg_call_err_locked');
  return '';
}

class Dialer {
  /** The question «Позвонить?» open now, for Dialer.svelte. */
  asking = $state<{ target: DialTarget; media: CallMedia; phone: boolean } | null>(null);

  /** Call `target`: between two after the question (unless it was turned off), a group at once. */
  call(target: DialTarget, media: CallMedia, phone: boolean) {
    const block = dialBlock();
    if (block) { toast.error(block); return; }
    if (target.kind === 'dm' && callStore.confirm) this.asking = { target, media, phone };
    else void this.go(target, media, phone);
  }

  async go(target: DialTarget, media: CallMedia, phone: boolean) {
    this.asking = null;
    const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'msg_title', params);
    if (target.kind === 'group') {
      const on = groupCallStore.announcedIn(target.peer);
      const { view, refusal } = on ? await groupCallStore.join(target.peer) : await groupCallStore.start(target.peer, media);
      if (view) openGroupRoom(phone);
      else if (refusal) toast.error(callErrorText(refusal, tr));
      return;
    }
    const { view, refusal } = await callStore.dial(target.address, media);
    if (!view && refusal) toast.error(callErrorText(refusal, tr));
  }
}

export const dialer = new Dialer();

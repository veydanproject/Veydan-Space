// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// My invitations to a chat with me, and the way in by someone else's. The
// runtime keeps the invitations and counts their uses; this store holds
// what it last said, and reads it again on an event, never taking the event
// for the state: a use arrives as a changed relationship, a pause as
// `dm.invite_paused`. A pause means someone tried ten wrong secrets within
// an hour: until it ends, every code and link of mine lets nobody in by
// itself, and the settings say so.

import { messengerApi, messengerError, type InvitePreset, type InviteView, type MessengerUiEvent } from '../api';
import type { InternalLinkText } from '../content/types';
import { chatStore } from '../chats/chatStore.svelte';
import { invitedContact, pausedUntil } from './invites';

/** Uses come in bursts (a history read on start): one read for each burst. */
const RELOAD_MS = 300;

/** The module store imports this one: reached late, as the chat store does. */
function refreshContacts() {
  import('../store.svelte').then((m) => m.messengerStore.refreshContacts()).catch(() => {});
}

/** What came of a scanned or pasted link of a person. */
export type JoinOutcome =
  | { ok: true; pubkey: string; name: string; invited: boolean }
  | { ok: false; reason: 'not_contact' | 'is_me' | 'blocked' };

class InviteStore {
  /** Mine, as the runtime lists them; `null` until read. */
  list = $state<InviteView[] | null>(null);
  /** Unix seconds the pause ends at, as the runtime said it; `null` when none was said. */
  pausedAt = $state<number | null>(null);
  error = $state('');
  /** The invitation being revoked: its button waits. */
  revoking = $state<string | null>(null);
  private _timer: ReturnType<typeof setTimeout> | null = null;
  /** Bumped by `reset`: an answer to a read started before it is of another session. */
  private _epoch = 0;

  /** Until when the invitations are paused at `now`; `null` when they are not. */
  pausedUntil(now = Math.floor(Date.now() / 1000)): number | null {
    return pausedUntil(this.pausedAt, now);
  }

  load = async () => {
    const epoch = this._epoch;
    try {
      const [list, status] = await Promise.all([messengerApi.moderation.inviteList(), messengerApi.moderation.inviteStatus()]);
      if (epoch !== this._epoch) return;
      this.list = list;
      this.pausedAt = status.paused_until ?? null;
      this.error = '';
    } catch (e) {
      if (epoch === this._epoch) this.error = messengerError(e);
    }
  };

  /** A new invitation; it joins the list at once, the runtime's order comes with the next read. */
  create = async (preset: InvitePreset): Promise<InviteView> => {
    const v = await messengerApi.moderation.inviteCreate(preset);
    this.list = [v, ...(this.list ?? []).filter((x) => x.id !== v.id)];
    return v;
  };

  revoke = async (id: string) => {
    this.error = '';
    this.revoking = id;
    try {
      await messengerApi.moderation.inviteRevoke(id);
      this.list = (this.list ?? []).map((v) => (v.id === id ? { ...v, revoked: true } : v));
      await this.load();
    } catch (e) {
      this.error = messengerError(e);
    } finally {
      this.revoking = null;
    }
  };

  /**
   * Into a chat by someone's link (scanned or pasted). With an invitation
   * in it, the request carries the secret and their device lets me in at
   * once; without one, the chat opens as with any key, and what I write
   * first is an ordinary request. Nothing is sent to my own link or to
   * someone I blocked.
   */
  join = async (text: string): Promise<JoinOutcome> => {
    const [view] = await messengerApi.links.inspect([text.trim() as InternalLinkText]);
    const c = invitedContact(view);
    if (!c) return { ok: false, reason: 'not_contact' };
    if (c.isMe) return { ok: false, reason: 'is_me' };
    if (c.blocked) return { ok: false, reason: 'blocked' };
    if (c.invite) await this.request(c.pubkey, c.invite);
    return { ok: true, pubkey: c.pubkey, name: c.name, invited: !!c.invite };
  };

  /** A request with the secret of an invitation, and with a code the first message that goes with it. */
  request = async (peer: string, invite: string, text?: string) => {
    await messengerApi.dm.action(peer, 'request', { invite, ...(text?.trim() ? { text: text.trim() } : {}) });
    await chatStore.loadChats();
    refreshContacts();
  };

  /** Runtime event → a read again. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    if (ev.name === 'dm.invite_paused') {
      const until = (ev.payload as { until?: unknown } | null)?.until;
      if (typeof until === 'number') this.pausedAt = until;
      if (this.list) this.scheduleReload();
      return;
    }
    // Someone came in by an invitation of mine: its uses changed. A sync of
    // my devices brings the uses of the others.
    if ((ev.name === 'dm.relationship' || ev.name === 'history.synced') && this.list) this.scheduleReload();
  }

  private scheduleReload() {
    if (this._timer) return;
    this._timer = setTimeout(() => {
      this._timer = null;
      this.load().catch(() => {});
    }, RELOAD_MS);
  }

  reset() {
    this._epoch += 1;
    if (this._timer) clearTimeout(this._timer);
    this._timer = null;
    this.list = null;
    this.pausedAt = null;
    this.error = '';
    this.revoking = null;
  }
}

export const inviteStore = new InviteStore();

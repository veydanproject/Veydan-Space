// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the other side learns of the user, as the settings panel sees it.
// Each switch works both ways, so turning one off also changes what this
// user sees of others: the open chat is read again after a change. The
// presence switch is one for all the user's devices: turned on or off on
// another device, it moves here too.
//
// Then who may write to the user at all: anyone (a stranger's first text
// waits in the requests) or only their contacts, how long an unanswered
// request is kept, and the people they blocked, each with a way back. The
// blocked come named by the runtime: a blocked person's profile is never
// asked for, so the list cannot wait for one.

import {
  messengerApi, messengerError, type BlockedPerson, type DmFrom, type DmSettings, type MessengerPrivacy, type MessengerUiEvent,
} from '../api';
import { chatStore } from '../chats/chatStore.svelte';
import { presenceStore } from '../presence/presenceStore.svelte';

class PrivacyStore {
  view = $state<MessengerPrivacy | null>(null);
  busy = $state(false);
  /** A failure of the last action, as the runtime worded it. */
  error = $state('');
  /** Who may write to me and how long a request is kept; `null` until read. */
  dm = $state<DmSettings | null>(null);
  dmBusy = $state(false);
  dmError = $state('');
  /** The people I blocked; `null` until read. */
  blocked = $state<BlockedPerson[] | null>(null);
  /** The key being unblocked: its button waits. */
  unblocking = $state<string | null>(null);
  blockedError = $state('');
  private _blockedTimer: ReturnType<typeof setTimeout> | null = null;

  load = async () => {
    try {
      this.view = await messengerApi.privacy.get();
      presenceStore.setSharing(this.view.presence);
    } catch (e) {
      this.error = messengerError(e);
    }
  };

  /** Apart from `load`: a runtime without these commands still shows the switches above. */
  loadDm = async () => {
    try {
      const [dm, blocked] = await Promise.all([messengerApi.moderation.dmSettings(), messengerApi.moderation.blocked()]);
      this.dm = dm;
      this.blocked = blocked.filter((b) => b.peer);
    } catch (e) {
      this.dmError = messengerError(e);
    }
  };

  setDmFrom = (dmFrom: DmFrom) => this.setDm({ dm_from: dmFrom });
  setKeepDays = (days: number) => this.setDm({ keep_days: days });

  /** Unblocked, the person's chat comes back as it was, and the list of chats is read again. */
  unblock = async (peer: string) => {
    this.blockedError = '';
    this.unblocking = peer;
    try {
      await messengerApi.dm.action(peer, 'unblock');
      this.blocked = (await messengerApi.moderation.blocked()).filter((b) => b.peer);
      chatStore.loadChats().catch(() => {});
    } catch (e) {
      this.blockedError = messengerError(e);
    } finally {
      this.unblocking = null;
    }
  };

  /** Each switch sends the other back as it is. */
  setReadReceipts = (on: boolean) => this.set(on, this.view?.presence ?? true);
  setPresence = (on: boolean) => this.set(this.view?.read_receipts ?? true, on);

  /** Runtime event → state. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    // Another device moved the presence key, maybe with the switch.
    if (ev.name === 'presence.epoch_changed' && this.view) this.load().catch(() => {});
    // Someone blocked or unblocked here, in a chat, or on another device of mine.
    if ((ev.name === 'dm.relationship' || ev.name === 'dm.request') && this.blocked) this.scheduleBlocked();
  }

  /** History brings relationships in bursts: one read of the blocked for each. */
  private scheduleBlocked() {
    if (this._blockedTimer) return;
    this._blockedTimer = setTimeout(() => {
      this._blockedTimer = null;
      messengerApi.moderation.blocked().then((b) => (this.blocked = b.filter((x) => x.peer))).catch(() => {});
    }, 300);
  }

  /** The settings go whole: the runtime keeps what it is given. */
  private async setDm(change: Partial<DmSettings>) {
    if (!this.dm) return;
    this.dmError = '';
    this.dmBusy = true;
    try {
      await messengerApi.moderation.dmSettingsSet({ ...this.dm, ...change });
      this.dm = await messengerApi.moderation.dmSettings();
    } catch (e) {
      this.dmError = messengerError(e);
    } finally {
      this.dmBusy = false;
    }
  }

  private async set(readReceipts: boolean, presence: boolean) {
    this.error = '';
    this.busy = true;
    try {
      this.view = await messengerApi.privacy.set(readReceipts, presence);
      chatStore.reloadWindow().catch(() => {});
      // Off, nobody is shown, whatever event comes late; on, the runtime
      // has what it heard.
      presenceStore.setSharing(this.view.presence);
      presenceStore.load().catch(() => {});
    } catch (e) {
      this.error = messengerError(e);
    } finally {
      this.busy = false;
    }
  }
}

export const privacyStore = new PrivacyStore();

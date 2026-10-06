// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the other side learns of the user, as the settings panel sees it.
// Each switch works both ways, so turning one off also changes what this
// user sees of others: the open chat is read again after a change. The
// presence switch is one for all the user's devices: turned on or off on
// another device, it moves here too.

import { messengerApi, messengerError, type MessengerPrivacy, type MessengerUiEvent } from '../api';
import { chatStore } from '../chats/chatStore.svelte';
import { presenceStore } from '../presence/presenceStore.svelte';

class PrivacyStore {
  view = $state<MessengerPrivacy | null>(null);
  busy = $state(false);
  /** A failure of the last action, as the runtime worded it. */
  error = $state('');

  load = async () => {
    try {
      this.view = await messengerApi.privacy.get();
      presenceStore.setSharing(this.view.presence);
    } catch (e) {
      this.error = messengerError(e);
    }
  };

  /** Each switch sends the other back as it is. */
  setReadReceipts = (on: boolean) => this.set(on, this.view?.presence ?? true);
  setPresence = (on: boolean) => this.set(this.view?.read_receipts ?? true, on);

  /** Runtime event → state. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    // Another device moved the presence key, maybe with the switch.
    if (ev.name === 'presence.epoch_changed' && this.view) this.load().catch(() => {});
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

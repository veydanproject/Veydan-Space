// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The way to the project's servers as the screens see it: one state, read
// by the network panel, by the offer above the chats and by the card of a
// bridge link.

import { messengerApi, messengerError, type MessengerUiEvent, type NetCheck, type NetMode, type NetStatus } from '../api';

class NetStore {
  status = $state<NetStatus | null>(null);
  busy = $state(false);
  /** A failure of the last action, as the runtime worded it. */
  error = $state('');
  /** What the last check by hand found. */
  check = $state<NetCheck | null>(null);

  /** The direct way is restricted, a bridge would help, and nobody answered yet. */
  get toOffer(): boolean {
    return Boolean(this.status?.available && this.status.offer && this.status.mode === 'off');
  }

  /** Runs an action; its failure goes to `error` and is also the answer, '' on success. */
  private async attempt(action: () => Promise<NetStatus | void>): Promise<string> {
    this.error = '';
    this.busy = true;
    try {
      const status = await action();
      if (status) this.status = status;
      return '';
    } catch (e) {
      const failure = messengerError(e) || 'error';
      this.error = failure;
      return failure;
    } finally {
      this.busy = false;
    }
  }

  private async run(action: () => Promise<NetStatus | void>): Promise<boolean> {
    return (await this.attempt(action)) === '';
  }

  load = () => this.run(() => messengerApi.net.status());
  setMode = (mode: NetMode) => {
    this.check = null;
    return this.run(() => messengerApi.net.setMode(mode));
  };
  addBridge = (bridge: string) => this.run(() => messengerApi.net.addBridge(bridge));
  /**
   * Adds a bridge from a card in a chat. The answer is the failure of this
   * call alone, '' when it was added: a card shows its own failure, never one
   * of the network panel's.
   */
  addBridgeFromCard = (bridge: string) => this.attempt(() => messengerApi.net.addBridge(bridge));
  removeBridge = (id: string) => this.run(() => messengerApi.net.removeBridge(id));
  runCheck = () =>
    this.run(async () => {
      this.check = await messengerApi.net.check();
      return messengerApi.net.status();
    });
  /** "Not now": the offer goes away for a while. */
  decline = () =>
    this.run(async () => {
      await messengerApi.net.dismissOffer();
      return messengerApi.net.status();
    });

  /** The runtime says the way changed, or that there is something to offer. */
  handleEvent(event: MessengerUiEvent) {
    if (event.name !== 'net') return;
    messengerApi.net.status().then((s) => { this.status = s; }).catch(() => {});
  }

  reset() {
    this.status = null;
    this.check = null;
    this.error = '';
  }
}

export const netStore = new NetStore();

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// My private servers as the settings list them (never their keys), each
// with the count of its chats and groups here. The runtime says
// `servers.updated` when one came or went — added here by a card, removed,
// or brought by the snapshot of another device of mine — and the list is
// read again; the event is a signal, not the state. The chat list and the
// headers name a chat's server from here («Частный · S1»), so the list is
// read when the messenger starts.

import { messengerApi, type MessengerUiEvent, type PrivateServerView } from '../api';
import { hostOf } from './serverAccept.svelte';

/** `UI_EVENT_SERVERS_UPDATED` of the runtime. */
export const SERVERS_UPDATED = 'servers.updated';

/** The name a server goes by in the app: its own, or its relay's host. */
export function nameOfServer(s: Pick<PrivateServerView, 'name' | 'relay'>): string {
  return s.name?.trim() || hostOf(s.relay);
}

class ServerStore {
  /** `null` until read. */
  list = $state<PrivateServerView[] | null>(null);

  load = async () => {
    try {
      this.list = await messengerApi.servers.list();
    } catch {
      this.list = this.list ?? [];
    }
  };

  /** A private server of mine by its id; `null` for one this device does not have (or before the list is read). */
  byId(id: string | null | undefined): PrivateServerView | null {
    if (!id) return null;
    return this.list?.find((s) => s.id === id) ?? null;
  }

  /** The name of the server of a chat, for its label; `''` when it is not known here. */
  nameOf(id: string | null | undefined): string {
    const s = this.byId(id);
    return s ? nameOfServer(s) : '';
  }

  /** Removed here and on every device of mine, with its private chats. */
  remove = async (id: string) => {
    await messengerApi.servers.remove(id);
    await this.load();
  };

  handleEvent(ev: MessengerUiEvent) {
    if (ev.name === SERVERS_UPDATED) void this.load();
  }

  reset() {
    this.list = null;
  }
}

export const serverStore = new ServerStore();

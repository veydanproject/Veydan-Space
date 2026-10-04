// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Live transfer progress, keyed by message. Fed by `transfer.progress`
// runtime events; bubbles read their own entry.

import { messengerApi, type MessengerTransferProgress, type MessengerUiEvent } from '../api';

class TransferStore {
  byMessage = $state<Record<string, MessengerTransferProgress>>({});

  get(messageId: string): MessengerTransferProgress | null {
    return this.byMessage[messageId] ?? null;
  }

  handleEvent(ev: MessengerUiEvent) {
    if (ev.name !== 'transfer.progress') return;
    const p = ev.payload as MessengerTransferProgress;
    if (!p?.message_id) return;
    this.byMessage = { ...this.byMessage, [p.message_id]: p };
  }

  /** State of a transfer that started before this view existed. */
  async hydrate(messageId: string) {
    if (this.byMessage[messageId]) return;
    const t = await messengerApi.media.transfer(messageId).catch(() => null);
    if (!t) return;
    this.byMessage = {
      ...this.byMessage,
      [messageId]: {
        transfer_id: t.id, message_id: t.message_id, chat_id: t.chat_id, direction: t.direction, status: t.status,
        done_bytes: t.done_bytes, total_bytes: t.size, failure_reason: t.failure_reason, local_path: t.local_path,
      },
    };
  }

  forget(messageId: string) {
    const { [messageId]: _gone, ...rest } = this.byMessage;
    this.byMessage = rest;
  }
}

export const transferStore = new TransferStore();

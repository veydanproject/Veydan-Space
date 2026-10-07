// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Live transfers of files, by transfer and by message. Fed by
// `transfer.progress` runtime events; read in full once from the runtime
// (`messenger_media_transfers`) for what started before this view existed.
// Bubbles read their own entry, the header chip and its list read them all.

import {
  messengerApi, transferProgress, type MessengerTransferProgress, type MessengerUiEvent, type TransferStatus,
} from '../api';
import { retriable } from './transferLabel';

/** A finished transfer stays this long after its last event, so its end is seen. */
export const DROP_AFTER_MS = 3000;
const FINISHED: TransferStatus[] = ['done', 'cancelled'];

export interface TransferCounts {
  up: number;
  down: number;
  failed: number;
  /** What "retry all" takes up: failed, waiting for its next attempt, or paused by the closing of the app. */
  retriable: number;
}

class TransferStore {
  /** By transfer id, oldest first. */
  byId = $state<Record<string, MessengerTransferProgress>>({});
  /** Message id to the id of its latest transfer. */
  private ofMessage = $state<Record<string, string>>({});
  /** The whole list was read from the runtime. */
  loaded = $state(false);
  private loading: Promise<void> | null = null;
  private timers = new Map<string, ReturnType<typeof setTimeout>>();
  /** Bumped by `reset`: a read started before it is let go. */
  private era = 0;

  /** Every transfer under way, paused or failed, newest first. */
  list = $derived(Object.values(this.byId).filter((p) => !FINISHED.includes(p.status)).reverse());

  counts = $derived.by((): TransferCounts => {
    const c = { up: 0, down: 0, failed: 0, retriable: 0 };
    for (const p of this.list) {
      if (p.status === 'failed') c.failed++;
      else c[p.direction]++;
      if (retriable(p)) c.retriable++;
    }
    return c;
  });

  get(messageId: string): MessengerTransferProgress | null {
    const id = this.ofMessage[messageId];
    return id ? (this.byId[id] ?? null) : null;
  }

  byTransfer(transferId: string): MessengerTransferProgress | null {
    return this.byId[transferId] ?? null;
  }

  handleEvent(ev: MessengerUiEvent) {
    if (ev.name !== 'transfer.progress') return;
    const p = ev.payload as MessengerTransferProgress;
    if (!p?.message_id || !p.transfer_id) return;
    this.put(p);
  }

  private put(p: MessengerTransferProgress) {
    this.byId = { ...this.byId, [p.transfer_id]: p };
    if (p.message_id && this.ofMessage[p.message_id] !== p.transfer_id) this.ofMessage = { ...this.ofMessage, [p.message_id]: p.transfer_id };
    const timer = this.timers.get(p.transfer_id);
    if (timer) clearTimeout(timer);
    this.timers.delete(p.transfer_id);
    if (FINISHED.includes(p.status)) this.timers.set(p.transfer_id, setTimeout(() => this.drop(p.transfer_id), DROP_AFTER_MS));
  }

  private drop(transferId: string) {
    this.timers.delete(transferId);
    const p = this.byId[transferId];
    if (!p) return;
    const { [transferId]: _gone, ...rest } = this.byId;
    this.byId = rest;
    if (p.message_id && this.ofMessage[p.message_id] === transferId) {
      const { [p.message_id]: _was, ...others } = this.ofMessage;
      this.ofMessage = others;
    }
  }

  /** Reads every transfer once; an event that came first is newer and stays. */
  load(): Promise<void> {
    const era = this.era;
    this.loading ??= messengerApi.media.transfers()
      .then((views) => {
        // Read before a reset: of a runtime that is gone.
        if (era !== this.era) return;
        for (const v of [...(views ?? [])].reverse()) {
          if (v.message_id && !this.byId[v.id]) this.put(transferProgress(v));
        }
        this.loaded = true;
      })
      .catch(() => { if (era === this.era) this.loading = null; });
    return this.loading;
  }

  /** State of a transfer that started before this view existed. */
  async hydrate(messageId: string) {
    await this.load();
    if (this.loaded || this.get(messageId)) return;
    // A runtime without the whole list: this message's alone.
    const v = await messengerApi.media.transfer(messageId).catch(() => null);
    if (v && !this.byId[v.id]) this.put(transferProgress(v));
  }

  /** A cancel of the transfer went through: its entry goes now, unless an event ended it already. One nothing ran may be told late. */
  cancelled(transferId: string) {
    const p = this.byId[transferId];
    if (p && !FINISHED.includes(p.status)) this.drop(transferId);
  }

  forget(messageId: string) {
    const id = this.ofMessage[messageId];
    if (id) this.drop(id);
  }

  /** The messenger stopped or its identity changed: nothing of before is kept, and the list is read again. */
  reset() {
    this.era++;
    for (const timer of this.timers.values()) clearTimeout(timer);
    this.timers.clear();
    this.byId = {};
    this.ofMessage = {};
    this.loaded = false;
    this.loading = null;
  }
}

export const transferStore = new TransferStore();

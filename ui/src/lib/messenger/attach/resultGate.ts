// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Who may hand a result to the composer. A panel (`done`) or an item that
// runs (`run`) answers late, twice, or after the sheet was closed, its tab
// left or the chat changed; the composer must take such an answer from no
// one. Each use of an item takes a ticket, and a ticket is good for one
// result, from the sheet, the tab (and the chat) it was taken in, and for as
// long as the registration it was taken from is alive: a module that stopped
// takes its answers back with it, and neither a later item of the same id nor
// the same object given again inherits any of them (platform-spec 6.3).

import type { AttachContext, AttachRegistration, AttachResult } from '../seams/attach.svelte';

export interface Ticket {
  readonly kind: 'panel' | 'run';
  readonly chat: string;
  readonly item: string;
  /** The registration itself: not the id, not the action object. */
  readonly reg: AttachRegistration;
  readonly sheet: number;
  readonly epoch: number;
  used: boolean;
}

export class ResultGate {
  /** `alive`: whether a registration still stands, asked at the moment a result arrives. */
  constructor(private readonly alive: (r: AttachRegistration) => boolean) {}

  private sheet = 0;
  // Moves when the composer's chat changes or the sheet is opened again or gone: a `run` outlives the closing of the sheet, but not that.
  private epoch = 0;
  private chat = '';
  private shown = false;
  private tab: string | null = null;

  open(chat: string) {
    this.sheet++;
    this.epoch++;
    this.chat = chat;
    this.shown = true;
    this.tab = null;
  }

  close() {
    this.sheet++;
    this.shown = false;
    this.tab = null;
  }

  /** The composer now belongs to another chat. */
  moveTo(chat: string) {
    if (chat === this.chat) return;
    this.chat = chat;
    this.sheet++;
    this.epoch++;
  }

  /** The component is gone. */
  destroy() {
    this.close();
    this.epoch++;
  }

  /** A panel of the tab `item` is shown. */
  show(item: string | null) {
    if (item === this.tab) return;
    this.tab = item;
    this.sheet++; // leaving a tab, even to come back to it, spends the tickets of the panel
  }

  panel(reg: AttachRegistration): Ticket {
    return { kind: 'panel', chat: this.chat, item: reg.action.id, reg, sheet: this.sheet, epoch: this.epoch, used: false };
  }

  run(reg: AttachRegistration): Ticket {
    return { kind: 'run', chat: this.chat, item: reg.action.id, reg, sheet: this.sheet, epoch: this.epoch, used: false };
  }

  /** Whether the ticket's result may go to the composer; the ticket is spent when it may. */
  take(t: Ticket): boolean {
    if (t.used || t.chat !== this.chat || t.epoch !== this.epoch || !this.alive(t.reg)) return false;
    if (t.kind === 'panel' && !(this.shown && t.sheet === this.sheet && t.item === this.tab)) return false;
    t.used = true;
    return true;
  }
}

export interface AttachSink {
  onresult: (result: AttachResult) => void;
  onerror?: (error: unknown) => void;
}

/** The answer of a panel: taken only if its ticket is good. Returns whether the sheet is to close. */
export function finishPanel(gate: ResultGate, t: Ticket, result: AttachResult | null, sink: AttachSink): boolean {
  if (!gate.take(t)) return false;
  if (result) sink.onresult(result);
  return true;
}

/** An item that runs: the answer and the failure both go through the ticket taken before it started. */
export async function runAction(gate: ResultGate, reg: AttachRegistration, ctx: AttachContext, sink: AttachSink): Promise<void> {
  const t = gate.run(reg);
  try {
    const result = await reg.action.run?.(ctx);
    if (result && gate.take(t)) sink.onresult(result);
  } catch (e) {
    if (gate.take(t)) sink.onerror?.(e);
  }
}

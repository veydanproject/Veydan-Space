// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the mode of choosing messages of a chat may take, and where a
// forward of them may go: the same rules as the runtime's
// (`messenger_runtime::forward`, `forward_message_unfit` and
// `forward_target_unfit`), so nothing it would refuse is ever offered.
// An album is chosen as a whole: one unit of what may be chosen.

import type { MessengerChat, MessengerForwardResult, MessengerMessage } from '../api';
import type { TimelineItem } from '../content/types';

/** The most messages one forward takes (`MAX_FORWARD_MESSAGES`). */
export const MAX_FORWARD_MESSAGES = 100;
/** The most chats one forward goes to (`MAX_FORWARD_TARGETS`). */
export const MAX_FORWARD_TARGETS = 10;

type Fit = Pick<MessengerMessage, 'id' | 'status' | 'content_type' | 'deleted' | 'review' | 'bot_card'>;

/**
 * A message that may be chosen and forwarded: a text, a file or a card, sent
 * or received, there, not waiting for a moderator nor turned down. Never a
 * line of the app, a call, a card of the bot, nor a type this app does not
 * know; never what is on its way, paused or failed.
 */
export function forwardable(m: Fit): boolean {
  return !m.deleted && !m.bot_card && !m.id.startsWith('local:')
    && (m.status === 'sent' || m.status === 'received')
    && (m.content_type === 'text' || m.content_type === 'media' || m.content_type === 'contact')
    && (m.review === undefined || m.review === 'ok');
}

/**
 * A chat a forward may go to: a direct chat with a contact of mine open both
 * ways, or a group of mine where I may write. Never the bot of Veydan Cloud,
 * a channel of Communities, a request, the archive, a blocked person.
 */
export function forwardTarget(c: MessengerChat): boolean {
  if (!c.can_send || c.archived || c.banned || c.bot) return false;
  if (c.kind === 'group') return c.id.startsWith('group:');
  return c.kind === 'dm' && c.id.startsWith('dm:') && c.is_contact && c.mode === 'full_chat';
}

/** One thing to tick: a bubble, or an album; `ids` its messages that may go, `key` the id of its row. */
export interface Unit {
  key: string;
  ids: string[];
}

/** What may be ticked in the timeline, in its order: a bubble or an album with at least one message that may go. */
export function selectUnits(items: readonly TimelineItem[]): Unit[] {
  const out: Unit[] = [];
  for (const item of items) {
    const parts = item.type === 'bubble' ? [item.message] : item.type === 'album' ? item.messages : [];
    const ids = parts.filter(forwardable).map((m) => m.id);
    if (ids.length) out.push({ key: item.id, ids });
  }
  return out;
}

/** What a tick changed: the chosen ids, and whether the limit held something back. */
export interface Ticked {
  chosen: string[];
  refused: boolean;
}

/** Ticks `unit` (or unticks it when all of it is chosen); an album that would pass `max` is refused whole. */
export function toggleUnit(chosen: readonly string[], unit: Unit, max = MAX_FORWARD_MESSAGES): Ticked {
  const has = new Set(chosen);
  if (unit.ids.every((id) => has.has(id))) return { chosen: chosen.filter((id) => !unit.ids.includes(id)), refused: false };
  return addUnits(chosen, [unit], max);
}

/** Ticks `units` in their order while `max` holds; one that does not fit whole is left, and so are the rest. */
export function addUnits(chosen: readonly string[], units: readonly Unit[], max = MAX_FORWARD_MESSAGES): Ticked {
  const out = [...chosen];
  const has = new Set(chosen);
  for (const u of units) {
    const fresh = u.ids.filter((id) => !has.has(id));
    if (out.length + fresh.length > max) return { chosen: out, refused: true };
    for (const id of fresh) { out.push(id); has.add(id); }
  }
  return { chosen: out, refused: false };
}

/** The units from `from` to `to` (row keys), both included, in the order of the timeline; none when either is gone. */
export function unitRange(units: readonly Unit[], from: string, to: string): Unit[] {
  const a = units.findIndex((u) => u.key === from);
  const b = units.findIndex((u) => u.key === to);
  if (a < 0 || b < 0) return [];
  return a <= b ? units.slice(a, b + 1) : units.slice(b, a + 1).reverse();
}

/** The words of the chosen messages, in the order of the chat, for the clipboard. */
export function chosenText(messages: readonly Pick<MessengerMessage, 'id' | 'text'>[], chosen: readonly string[]): string {
  const has = new Set(chosen);
  return messages.filter((m) => has.has(m.id) && m.text?.trim()).map((m) => m.text!.trim()).join('\n\n');
}

/** What a forward came to: the chats that got all of it, and what the others did not get. */
export interface ForwardOutcome {
  done: string[];
  /** A chat that got less: why the rest did not go (`error`), and the files it could not get (`skipped`, their reasons). */
  short: { target: string; error: string | null; skipped: string[] }[];
}

export function forwardOutcome(results: readonly MessengerForwardResult[]): ForwardOutcome {
  const out: ForwardOutcome = { done: [], short: [] };
  for (const r of results) {
    const skipped = (r.skipped ?? []).map((s) => s.reason);
    if (!r.error && !skipped.length) out.done.push(r.target);
    else out.short.push({ target: r.target, error: r.error ?? null, skipped });
  }
  return out;
}

/** The openings of the sheet of a forward: an answer that comes after its
 * sheet was closed (or opened again for other messages) is only told, it
 * closes nothing and clears nothing of the new opening. */
export class ForwardSessions {
  private n = 0;
  /** The sheet opened or closed: what was started before is stale. */
  turn(): void {
    this.n++;
  }
  /** The opening a send starts in. */
  start(): number {
    return this.n;
  }
  /** Whether a send of `opening` still stands in its own sheet. */
  live(opening: number): boolean {
    return opening === this.n;
  }
}

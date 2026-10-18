// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The decisions of the queue, for a card of an author and for the screen
// of their messages alike: what a decision sends, and every waiting
// message of an author under the filter, which "approve all" decides.

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { confirmStore } from '../../shared/confirm.svelte';
import { messengerApi, type ModAction, type QueueFilter, type QueueItem } from '../../api';

export type Decision = 'approve' | 'reject' | 'spam';

/** A page of an author's messages: a flood is read in a few of them. */
const PAGE = 500;

/** The acts a decision sends: one per message, a spam ban one per author. */
export function actionsFor(rows: QueueItem[], kind: Decision): ModAction[] {
  if (kind === 'approve') return rows.map((r) => ({ t: 'approve', target: r.message_id, version: r.version }));
  if (kind === 'reject') return rows.map((r) => ({ t: 'reject', target: r.message_id }));
  const authors = [...new Set(rows.map((r) => r.author))];
  return authors.map((a) => ({ t: 'spam', author: a, target: rows.find((r) => r.author === a)?.message_id }));
}

/**
 * Asked before any decision: the buttons stand side by side, and a tap
 * that missed must not approve a flood or ban a member. `n` is how many
 * messages the decision takes.
 */
export function confirmDecision(kind: Decision, names: string[], n: number): Promise<boolean> {
  const tr = get(t);
  const who = names.join(', ');
  if (kind === 'spam') return confirmStore.ask(tr('msg_mod_spam_confirm', { names: who }), tr('msg_mod_spam'), true);
  const text = tr(kind === 'approve' ? 'msg_mod_approve_confirm' : 'msg_mod_reject_confirm', { names: who, n: String(n) });
  return confirmStore.ask(text, tr(kind === 'approve' ? 'msg_mod_approve' : 'msg_mod_reject'), kind === 'reject');
}

/** Every message of `author` waiting under `filter`, newest first. */
export async function waitingOf(group: string, filter: QueueFilter, author: string): Promise<QueueItem[]> {
  const out: QueueItem[] = [];
  const seen = new Set<string>();
  let before: number | undefined;
  for (;;) {
    const page = await messengerApi.moderation.groupQueue(group, { ...filter, author, each: true }, before, PAGE);
    for (const x of page.items) if (!seen.has(x.message_id)) { seen.add(x.message_id); out.push(x); }
    if (page.next_before === undefined || page.next_before === before) return out;
    before = page.next_before;
  }
}

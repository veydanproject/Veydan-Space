// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/moderation/queue.ts at 8ce09c39, then changed for channels.

// The decisions of the queue of a channel: what a decision sends, and the
// question asked before it. The queue shows one card per author
// (`QueueItem.more_from_author` counts the rest); a decision on a card
// takes everything of its author, read anew (`channel_moderation_queue`
// with `author`, one line per message), so what came after the card was
// drawn is decided too.

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { confirmStore } from '$lib/messenger/seams/shared';
import { channelsApi, type ModAction, type QueueItem } from '../api';

export type Decision = 'approve' | 'reject' | 'spam';

/** The most one author's page holds: the bound of the command. */
const AUTHOR_PAGE = 500;

/** Everything of one author that waits, newest first. */
export async function waitingOf(communityId: string, author: string): Promise<QueueItem[]> {
  return (await channelsApi.moderation.queue(communityId, undefined, AUTHOR_PAGE, author)).items;
}

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
  const tr = get(t) as (key: string, params?: Record<string, string>) => string;
  const who = names.join(', ');
  if (kind === 'spam') return confirmStore.ask(tr('chan_mod_spam_confirm', { names: who }), tr('chan_mod_spam'), true);
  const text = tr(kind === 'approve' ? 'chan_mod_approve_confirm' : 'chan_mod_reject_confirm', { names: who, n: String(n) });
  return confirmStore.ask(text, tr(kind === 'approve' ? 'chan_mod_approve' : 'chan_mod_reject'), kind === 'reject');
}


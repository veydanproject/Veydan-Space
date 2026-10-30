// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The menu of the head of a channel's feed: its info, its moderation for
// the team, the choices of its chat (pin, mute, archive), a report on the
// channel, and the way out. The page gives the acts; this file decides
// which are offered.
//
// A channel is reported on what it publishes, by anyone who sees it — its
// owner and its team too (Google Play: every user can report what is
// published, 2026-10-22); a post or a comment by anyone but its author
// (itemMenu.ts).

import type { MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
import type { ChannelDetail, ChannelView } from './api';
import { isModerator, isOwner, joined } from './permissions';

type Translate = (key: string, params?: Record<string, string>) => string;

/** The acts behind the entries. */
export interface ChannelActions {
  info: () => void;
  moderation: () => void;
  pin: () => void;
  mute: () => void;
  archive: () => void;
  report: () => void;
  cancelJoin: () => void;
  leave: () => void;
  forget: () => void;
}

/**
 * What waits for the team, said in the head menu (on every screen, the ⋮
 * carrying their sum): each count with the way to its screen.
 */
export interface StaffCounts {
  queue: number;
  requests: number;
  levels: number;
  openQueue: () => void;
  openRequests: () => void;
  openLevels: () => void;
}

/** The rows of the counts that are not zero, first in the menu. */
export function countEntries(c: StaffCounts, tr: Translate): MenuEntry[] {
  const rows: MenuEntry[] = [];
  if (c.queue > 0) rows.push({ label: tr('chan_menu_queue'), badge: String(c.queue), icon: 'shield', onselect: c.openQueue });
  if (c.requests > 0) rows.push({ label: tr('chan_mod_tab_requests'), badge: String(c.requests), icon: 'user-plus', onselect: c.openRequests });
  if (c.levels > 0) rows.push({ label: tr('chan_menu_level_requests'), badge: String(c.levels), icon: 'layers', onselect: c.openLevels });
  return rows;
}

/** The entries of the head menu of `row`; `waitingKey`: a key is still awaited; `counts`: what waits for the team (null: none to show). */
export function channelEntries(
  row: Pick<ChannelView, 'pinned' | 'muted' | 'archived'>, d: ChannelDetail | null | undefined, waitingKey: boolean, tr: Translate, a: ChannelActions,
  counts: StaffCounts | null = null,
): MenuEntry[] {
  const waiting = counts ? countEntries(counts, tr) : [];
  const list: MenuEntry[] = [...waiting, ...(waiting.length ? [{ type: 'separator' } as const] : []), { label: tr('chan_info'), icon: 'info', onselect: a.info }];
  if (isModerator(d)) list.push({ label: tr('chan_mod_title'), icon: 'shield', onselect: a.moderation });
  list.push(
    { type: 'separator' },
    { label: tr(row.pinned ? 'chan_unpin_chat' : 'chan_pin_chat'), icon: 'pin', onselect: a.pin },
    { label: tr(row.muted ? 'chan_unmute' : 'chan_mute'), icon: row.muted ? 'bell' : 'bell-off', onselect: a.mute },
    { label: tr(row.archived ? 'chan_unarchive' : 'chan_archive'), icon: row.archived ? 'archive-restore' : 'archive', onselect: a.archive },
    { type: 'separator' },
    { label: tr('chan_report_channel'), icon: 'alert-triangle', danger: true, onselect: a.report },
  );
  if (waitingKey && !isOwner(d)) list.push({ label: tr('chan_cancel_join'), icon: 'x', danger: true, onselect: a.cancelJoin });
  else if (joined(d) && !isOwner(d)) list.push({ label: tr('chan_leave'), icon: 'x', danger: true, onselect: a.leave });
  else if (!joined(d)) list.push({ label: tr('chan_forget'), icon: 'trash-2', danger: true, onselect: a.forget });
  return list;
}

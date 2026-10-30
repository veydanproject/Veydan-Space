// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The menu of a post or a comment, the same in the feed and in a thread:
// what anyone may do with it (comment, answer, copy, write to its author,
// report it), what its author may (edit, remove), and what a moderator may
// (pin, hide, let a waiting one through or turn it down, remove one of a
// role below). The page gives the acts; this file only decides which are
// offered, by the rules of permissions.ts.

import type { MenuEntry, MenuItem } from '$lib/core/ui/ContextMenu.svelte';
import type { ChannelDetail, FeedItemView } from './api';
import { canComment, canDelete, canEdit, canModerate, canPin, isModerator, mutedNow, staffOf } from './permissions';

type Translate = (key: string, params?: Record<string, string>) => string;

/** The acts behind the entries; an act left out is not offered. */
export interface ItemActions {
  comment?: () => void;
  reply?: () => void;
  copy: () => void;
  pin?: (pinned: boolean) => void;
  edit?: () => void;
  remove: () => void;
  hide?: () => void;
  approve?: () => void;
  reject?: () => void;
  /** Everything the author wrote in a span, hidden (a span is asked). */
  hideAuthor?: () => void;
  /** The author kept from writing for a span (asked). */
  mute?: () => void;
  /** The author banned (asked, with the hiding of what they wrote). */
  ban?: () => void;
  /** A post that waits: its author banned as a spammer, all theirs hidden. */
  spam?: () => void;
  write: () => void;
  report: () => void;
}

/**
 * The entries of the menu of `item` for me (`me`, hex), in their order, in groups
 * a line apart (the mockup c6.js, variant A): the frequent ones (the post, its words,
 * its author), the moderator's under their title «Модерация» (a waiting one's verdict
 * a pair of buttons), then the dangerous ones.
 */
export function itemEntries(item: FeedItemView, d: ChannelDetail, me: string | null, tr: Translate, a: ItemActions): MenuEntry[] {
  if (item.deleted) return [];
  const list: MenuItem[] = [];
  const review: MenuItem[] = [];
  const author: MenuItem[] = [];
  const mod: MenuItem[] = [];
  const isPost = !item.post_id;
  const others = item.author !== me;
  if (isPost && a.comment && (d.comments === 'on' || item.comments > 0)) {
    list.push({ label: tr('chan_menu_comments'), icon: 'message-circle', onselect: a.comment });
  }
  if (!isPost && a.reply && canComment(d, me)) list.push({ label: tr('chan_reply'), icon: 'reply', onselect: a.reply });
  if (item.text) list.push({ label: tr('chan_menu_copy'), icon: 'copy', onselect: a.copy });
  if (isPost && a.pin && canPin(d) && item.review !== 'pending') {
    list.push({ label: tr(item.pinned ? 'chan_menu_unpin' : 'chan_menu_pin'), icon: 'pin', onselect: () => a.pin?.(!item.pinned) });
  }
  if (item.review === 'pending' && isModerator(d) && others) {
    if (a.approve) review.push({ label: tr('chan_mod_approve'), icon: 'check', onselect: a.approve });
    if (a.reject) review.push({ label: tr('chan_mod_reject'), icon: 'x', onselect: a.reject });
  }
  // What a moderator does to the author from their message, as from the team (and as a group's chat does).
  const below = others && canModerate(d, item.author, me);
  if (a.hide && !item.hidden && below) mod.push({ label: tr('chan_menu_hide'), icon: 'eye-off', onselect: a.hide });
  if (below && a.hideAuthor) mod.push({ label: tr('chan_mod_hide_author'), icon: 'eye-off', onselect: a.hideAuthor });
  if (below && a.mute && !mutedNow(staffOf(d, item.author))) mod.push({ label: tr('chan_member_mute'), icon: 'bell-off', onselect: a.mute });
  if (a.edit && canEdit(item, me)) author.push({ label: tr('chan_menu_edit'), icon: 'pencil', onselect: a.edit });
  // «Написать автору» is a message, not the comments: an icon of its own.
  if (others) author.push({ label: tr('chan_menu_write'), icon: 'send', onselect: a.write });
  const danger: MenuItem[] = [];
  if (canDelete(d, item, me)) danger.push({ label: tr('chan_menu_delete'), icon: 'trash-2', danger: true, onselect: a.remove });
  if (below && item.review === 'pending' && a.spam) danger.push({ label: tr('chan_mod_spam'), icon: 'ban', danger: true, onselect: a.spam });
  else if (below && a.ban) danger.push({ label: tr('chan_member_ban'), icon: 'user-cancel', danger: true, onselect: a.ban });
  if (others) danger.push({ label: tr('chan_menu_report'), icon: 'alert-triangle', danger: true, onselect: a.report });
  // The moderator's group: its title, the verdict a pair of buttons (one alone stays a row), then the rest.
  const moderation: MenuEntry[] = [];
  if (review.length || mod.length) {
    moderation.push({ type: 'heading', label: tr('chan_mod_title'), icon: 'shield' });
    if (review.length === 2) moderation.push({ type: 'pair', items: [review[0], review[1]] });
    else moderation.push(...review);
    moderation.push(...mod);
  }
  const out: MenuEntry[] = [];
  for (const group of [[...list, ...author], moderation, danger] as MenuEntry[][]) {
    if (!group.length) continue;
    if (out.length) out.push({ type: 'separator' });
    out.push(...group);
  }
  return out;
}

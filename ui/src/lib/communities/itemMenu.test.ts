// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { ChannelDetail, FeedItemView } from './api';
import { itemEntries, type ItemActions } from './itemMenu';

const ME = 'aa'.repeat(32);
const tr = (key: string) => key;
const noop = () => {};
const acts: ItemActions = {
  comment: noop, reply: noop, copy: noop, pin: noop, edit: noop, remove: noop, hide: noop, approve: noop, reject: noop, write: noop, report: noop,
  hideAuthor: noop, mute: noop, ban: noop, spam: noop,
};
const detail = (role: string | null): ChannelDetail => ({
  channel: {
    id: 'c', chat_id: 'channel:c', name: 'C', about: '', owner: 'o', join: 'open', membership: 'joined', my_role: role ?? undefined, members: 1,
    relays: [], storage: [], link_epoch: 1, kept_here: false, needs_update: false, unread: 0, pinned: false, muted: false, archived: false, created_at: 0, updated_at: 0, last_at: 0,
  },
  history: 'open', post_mode: 'authors', comments: 'on', premod: 'off', pinned: [], disbanded: false, has_key: true, waiting: 0,
  staff: role ? [{ pubkey: ME, role, muted: false, trusted: false, since: 0 }, { pubkey: 'w', role: 'author', muted: false, trusted: false, since: 0 }] : [],
});
const post = (author: string, extra: Partial<FeedItemView> = {}): FeedItemView => ({
  id: 'p', community_id: 'c', post_id: '', author, created_at: 1, text: 'hi', review: 'ok', hidden: false, deleted: false, pinned: false,
  reactions: [], comments: 0, pending: false, ...extra,
});
const labels = (d: ChannelDetail, item: FeedItemView) =>
  itemEntries(item, d, ME, tr, acts).map((e) => ('label' in e ? e.label : '—'));

describe('the menu of a post', () => {
  it('offers a reader the comments, the copy, a message to the author and a report', () => {
    expect(labels(detail(null), post('w'))).toEqual(['chan_menu_comments', 'chan_menu_copy', 'chan_menu_write', '—', 'chan_menu_report']);
  });

  it('offers the author of a post its edit and its removal', () => {
    expect(labels(detail('author'), post(ME))).toEqual(['chan_menu_comments', 'chan_menu_copy', 'chan_menu_edit', '—', 'chan_menu_delete']);
  });

  it('offers a moderator the pin, the hiding, the author\'s mute and ban and the removal of a post below (ревью UX п. 18)', () => {
    expect(labels(detail('moderator'), post('w'))).toEqual([
      'chan_menu_comments', 'chan_menu_copy', 'chan_menu_pin', 'chan_mod_hide_author', 'chan_member_mute', 'chan_menu_write', '—',
      'chan_menu_hide', 'chan_menu_delete', 'chan_member_ban', 'chan_menu_report',
    ]);
  });

  it('offers spam, not a ban, on a post that waits, and nothing of the kind on one of a role not below', () => {
    expect(labels(detail('moderator'), post('w', { review: 'pending' }))).toContain('chan_mod_spam');
    expect(labels(detail('moderator'), post('w', { review: 'pending' }))).not.toContain('chan_member_ban');
    expect(labels(detail('author'), post('w'))).not.toContain('chan_member_mute');
  });

  it('offers a moderator the decision on a post that waits, and no pin for it', () => {
    expect(labels(detail('moderator'), post('w', { review: 'pending' }))).toContain('chan_mod_approve');
    expect(labels(detail('moderator'), post('w', { review: 'pending' }))).not.toContain('chan_menu_pin');
  });

  it('offers nothing for a post removed', () => {
    expect(labels(detail('owner'), post('w', { deleted: true }))).toEqual([]);
  });

  it('offers an answer under a comment to whoever may comment', () => {
    expect(labels(detail(null), post('w', { post_id: 'p0' }))).toContain('chan_reply');
  });
});

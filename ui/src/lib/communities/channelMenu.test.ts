// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { ChannelDetail, FeedItemView } from './api';
import { channelEntries, type ChannelActions, type StaffCounts } from './channelMenu';
import { itemEntries, type ItemActions } from './itemMenu';

const ME = 'aa'.repeat(32);
const tr = (key: string) => key;
const noop = () => {};
const acts: ChannelActions = { info: noop, moderation: noop, pin: noop, mute: noop, archive: noop, report: noop, cancelJoin: noop, leave: noop, forget: noop };
const itemActs: ItemActions = { copy: noop, remove: noop, write: noop, report: noop, comment: noop };
const detail = (role: string | null, owner = role === 'owner' ? ME : 'o'): ChannelDetail => ({
  channel: {
    id: 'c', chat_id: 'channel:c', name: 'C', about: '', owner, join: 'open', membership: 'joined', my_role: role ?? undefined, members: 1,
    relays: [], storage: [], link_epoch: 1, kept_here: false, needs_update: false, unread: 0, pinned: false, muted: false, archived: false, created_at: 0, updated_at: 0, last_at: 0,
  },
  history: 'open', post_mode: 'authors', comments: 'on', premod: 'off', pinned: [], disbanded: false, has_key: true, waiting: 0,
  staff: role ? [{ pubkey: ME, role, muted: false, trusted: false, since: 0 }] : [],
});
const post = (author: string): FeedItemView => ({
  id: 'p', community_id: 'c', post_id: '', author, created_at: 1, text: 'hi', review: 'ok', hidden: false, deleted: false, pinned: false, reactions: [], comments: 0, pending: false,
});
const labels = (d: ChannelDetail) => channelEntries(d.channel, d, false, tr, acts).map((e) => ('label' in e ? e.label : '—'));
const itemLabels = (d: ChannelDetail, author: string) => itemEntries(post(author), d, ME, tr, itemActs).map((e) => ('label' in e ? e.label : '—'));

describe('«Пожаловаться» is there for everyone (Play, 2026-10-22)', () => {
  it('offers a report on the channel to its owner, its team and its readers alike', () => {
    for (const role of ['owner', 'admin', 'moderator', 'author', null]) expect(labels(detail(role))).toContain('chan_report_channel');
  });

  it('keeps the way out after the report, and none for the owner', () => {
    expect(labels(detail(null)).slice(-2)).toEqual(['chan_report_channel', 'chan_leave']);
    expect(labels(detail('owner')).at(-1)).toBe('chan_report_channel');
    expect(labels(detail('moderator'))).toContain('chan_mod_title');
  });

  it('offers a report on a post of somebody else to the owner and the team too, never on one of mine', () => {
    for (const role of ['owner', 'moderator', 'author', null]) {
      expect(itemLabels(detail(role), 'w')).toContain('chan_menu_report');
      expect(itemLabels(detail(role), ME)).not.toContain('chan_menu_report');
    }
  });
});

describe('the counts of the team in the menu (a phone, 2026-10-23)', () => {
  const counts = (queue: number, requests: number, levels: number): StaffCounts => ({ queue, requests, levels, openQueue: noop, openRequests: noop, openLevels: noop });
  const withCounts = (c: StaffCounts | null) => channelEntries(detail('owner').channel, detail('owner'), false, tr, acts, c).map((e) => ('label' in e ? e.label : '—'));

  it('come first, each with its number, then a line', () => {
    expect(withCounts(counts(4, 2, 2)).slice(0, 5)).toEqual(['chan_menu_queue 4', 'chan_mod_tab_requests 2', 'chan_menu_level_requests 2', '—', 'chan_info']);
  });

  it('leave out what is zero, and say nothing when all are', () => {
    expect(withCounts(counts(0, 3, 0)).slice(0, 3)).toEqual(['chan_mod_tab_requests 3', '—', 'chan_info']);
    expect(withCounts(counts(0, 0, 0))[0]).toBe('chan_info');
    expect(withCounts(null)[0]).toBe('chan_info');
  });
});

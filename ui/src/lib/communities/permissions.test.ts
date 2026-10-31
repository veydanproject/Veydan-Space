// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/permissions.test.ts at 8ce09c39, then changed for channels.

import { describe, expect, it } from 'vitest';
import type { ChannelDetail, StaffView } from './api';
import {
  KEEPER_AWAY_SECS, canAttach, catchUpNote, canComment, canRecord, canDelete, canEdit, canModerate, canPost, canSetUpSections, isManager, isModerator, keeperAwayAt, losesModeration, mutedNow, rolesFor, standing,
} from './permissions';

const ME = 'aa'.repeat(32);
const staff = (pubkey: string, role: string, over: Partial<StaffView> = {}): StaffView =>
  ({ pubkey, role, muted: false, trusted: false, since: 1, ...over });

/** A channel where I hold `mine` (a reader: none) beside one of each role of the team. */
function channel(mine: string | null, over: Partial<ChannelDetail> = {}, membership = 'joined'): ChannelDetail {
  return {
    channel: {
      id: 'c', chat_id: 'channel:c', name: 'C', about: '', owner: 'owner', join: 'open', membership, my_role: mine ?? undefined, members: 10,
      relays: [], storage: [], link_epoch: 1, kept_here: false, needs_update: false, unread: 0, pinned: false, muted: false, archived: false,
      created_at: 0, updated_at: 0, last_at: 0,
    },
    history: 'open', post_mode: 'authors', comments: 'on', premod: 'off', pinned: [], disbanded: false, has_key: true, waiting: 0, waiting_keeper: 0, keeper_refused: false,
    staff: [
      ...(mine && mine !== 'member' ? [staff(ME, mine)] : []),
      staff('owner', 'owner'), staff('admin', 'admin'), staff('mod', 'moderator'), staff('author', 'author'),
    ],
    ...over,
  };
}

describe('who posts and who comments', () => {
  it('lets authors and the team post, a reader never (a post_mode of an older build changes nothing)', () => {
    expect(canPost(channel('author'), ME)).toBe(true);
    expect(canPost(channel('owner'), ME)).toBe(true);
    expect(canPost(channel(null), ME)).toBe(false);
    expect(canPost(channel(null, { post_mode: 'all' }), ME)).toBe(false);
  });

  it('lets a reader post when a level of theirs lets post into a section (2026-10-23)', () => {
    const sc = (scope_id: string, post: boolean) => ({ scope_id, title: '', scheme: 'sd-v1' as const, history: 'open' as const, mine: true, post });
    expect(canPost(channel(null, { scopes: [sc('everyone', false), sc('premium', true)] }), ME)).toBe(true);
    expect(canPost(channel(null, { scopes: [sc('everyone', false), sc('premium', false)] }), ME)).toBe(false);
  });

  it('lets anyone in comment while comments are on', () => {
    expect(canComment(channel(null), ME)).toBe(true);
    expect(canComment(channel(null, { comments: 'off' }), ME)).toBe(false);
    expect(canComment(channel(null, {}, 'requested'), ME)).toBe(false);
  });

  it('lets nobody write without a key, nor while muted', () => {
    expect(canPost(channel('author', { has_key: false }), ME)).toBe(false);
    expect(canComment(channel(null, { has_key: false }), ME)).toBe(false);
    const muted = channel(null, { staff: [staff(ME, 'member', { muted: true })] });
    expect(canComment(muted, ME)).toBe(false);
    const ended = channel(null, { staff: [staff(ME, 'member', { muted: true, muted_until: 1 })] });
    expect(canComment(ended, ME)).toBe(true);
  });

  it('lets authors and above attach files, a reader comments in words', () => {
    expect(canAttach(channel('author'))).toBe(true);
    expect(canAttach(channel(null))).toBe(false);
  });

  it('lets a reader record a voice message or a circle as a comment, never as a post (2026-10-23)', () => {
    expect(canRecord(channel(null), ME, 'comment')).toBe(true);
    expect(canRecord(channel(null), ME, 'post')).toBe(false);
    expect(canRecord(channel('author'), ME, 'post')).toBe(true);
    expect(canRecord(channel(null, { comments: 'off' }), ME, 'comment')).toBe(false);
    expect(canRecord(channel(null, { staff: [staff(ME, 'member', { muted: true })] }), ME, 'comment')).toBe(false);
  });
});

describe('the moderation a role is offered', () => {
  it('lets a moderator and above act on someone below, never a peer, a superior or themselves', () => {
    expect(canModerate(channel('moderator'), 'author', ME)).toBe(true);
    expect(canModerate(channel('moderator'), 'reader', ME)).toBe(true);
    expect(canModerate(channel('moderator'), 'mod', ME)).toBe(false);
    expect(canModerate(channel('moderator'), 'admin', ME)).toBe(false);
    expect(canModerate(channel('admin'), 'mod', ME)).toBe(true);
    expect(canModerate(channel('owner'), ME, ME)).toBe(false);
    expect(canModerate(channel('author'), 'reader', ME)).toBe(false);
  });

  it('knows the moderators and the managers', () => {
    expect(isModerator(channel('moderator'))).toBe(true);
    expect(isModerator(channel('author'))).toBe(false);
    expect(isManager(channel('admin'))).toBe(true);
    expect(canSetUpSections(channel('admin'))).toBe(true);
    expect(canSetUpSections(channel('moderator'))).toBe(false);
    expect(isManager(channel('moderator'))).toBe(false);
    expect(isModerator(channel('owner', {}, 'out'))).toBe(false);
  });

  it('gives roles below one\'s own, and admin: an admin makes admins too (2026-10-25)', () => {
    expect(rolesFor(channel('owner'), 'reader', ME)).toEqual(['author', 'moderator', 'admin']);
    expect(rolesFor(channel('admin'), 'reader', ME)).toEqual(['author', 'moderator', 'admin']);
    expect(rolesFor(channel('admin'), 'author', ME)).toEqual(['member', 'moderator', 'admin']);
    expect(rolesFor(channel('admin'), 'mod', ME)).toEqual(['member', 'author', 'admin']);
    expect(rolesFor(channel('moderator'), 'reader', ME)).toEqual([]);
    expect(rolesFor(channel('admin'), 'admin', ME)).toEqual([]);
    expect(rolesFor(channel('admin'), 'owner', ME)).toEqual([]);
  });

  it('lets the author edit their words, and remove what is theirs or below', () => {
    const post = { author: ME, deleted: false, media_json: undefined };
    expect(canEdit(post, ME)).toBe(true);
    const withFile = { ...post, media_json: '{}' };
    expect(canEdit(withFile, ME)).toBe(true);
    expect(canEdit({ ...post, deleted: true }, ME)).toBe(false);
    expect(canEdit({ ...post, author: 'author' }, ME)).toBe(false);
    expect(canDelete(channel(null), post, ME)).toBe(true);
    expect(canDelete(channel(null), { author: 'author' }, ME)).toBe(false);
    expect(canDelete(channel('moderator'), { author: 'author' }, ME)).toBe(true);
  });
});

describe('a mute and a moderator who stops being one', () => {
  it('ends a mute for a time by itself', () => {
    expect(mutedNow(staff('x', 'member', { muted: true }), 100)).toBe(true);
    expect(mutedNow(staff('x', 'member', { muted: true, muted_until: 200 }), 100)).toBe(true);
    expect(mutedNow(staff('x', 'member', { muted: true, muted_until: 50 }), 100)).toBe(false);
    expect(mutedNow(staff('x', 'member'), 100)).toBe(false);
  });

  it('offers to keep the decisions when a moderator falls below one', () => {
    expect(losesModeration('moderator', 'author')).toBe(true);
    expect(losesModeration('admin', null)).toBe(true);
    expect(losesModeration('admin', 'moderator')).toBe(false);
    expect(losesModeration('author', null)).toBe(false);
  });
});

describe('where I stand in a channel', () => {
  it('reads, waits for the key, or catches up with an old one', () => {
    expect(standing(channel(null))).toBe('reading');
    expect(standing(channel(null, { has_key: false }))).toBe('waiting_key');
    expect(standing(channel(null, { waiting: 3 }))).toBe('catching_up');
  });

  it('says «when the keeper is online» only of what waits for the keeper (ROO-270)', () => {
    expect(catchUpNote(channel(null))).toBeNull();
    // Epochs ahead, a shard, a snapshot: they come by themselves.
    expect(catchUpNote(channel(null, { waiting: 19 }))).toEqual({ key: 'chan_state_syncing', n: 19 });
    // Some wait for the keeper: those alone are counted under its text.
    expect(catchUpNote(channel(null, { waiting: 19, waiting_keeper: 4 }))).toEqual({ key: 'chan_state_catching_up', n: 4 });
  });

  it('is outside in each of the ways', () => {
    for (const m of ['joining', 'requested', 'rejected', 'out', 'none'] as const) expect(standing(channel(null, {}, m))).toBe(m);
    expect(standing(channel(null, { disbanded: true }))).toBe('disbanded');
  });
});

describe('a keeper away (ревью UX п. 1)', () => {
  const N = 1_000_000;
  it('is said only after a wait, and only when the keeper is unheard of as long', () => {
    expect(keeperAwayAt({ updated_at: N - 60 }, N)).toBe(false);
    expect(keeperAwayAt({ updated_at: N - KEEPER_AWAY_SECS }, N)).toBe(true);
    expect(keeperAwayAt({ updated_at: N - 2 * KEEPER_AWAY_SECS, keeper_last_seen: N - 60 }, N)).toBe(false);
    expect(keeperAwayAt({ updated_at: N - 2 * KEEPER_AWAY_SECS, keeper_last_seen: N - 3 * 86400 }, N)).toBe(true);
  });
});

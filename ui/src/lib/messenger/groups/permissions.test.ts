// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { GroupRole, MessengerGroup, MessengerGroupMember } from '../api';
import { answersRequests, canApprove, canHide, canSpam, canTrust, isModerator, losesModeration, mutedNow, seesModeration } from './permissions';

const ME = 'aa'.repeat(32);
const member = (pubkey: string, role: GroupRole, over: Partial<MessengerGroupMember> = {}): MessengerGroupMember =>
  ({ pubkey, role, muted: false, muted_until: null, trusted: false, joined_at: 1, is_me: pubkey === ME, ...over });

/** A group where I hold `mine` beside one of each other role. */
function group(mine: GroupRole | null, over: Partial<MessengerGroup> = {}): MessengerGroup {
  return {
    id: 'g', chat_id: 'group:g', kind: 'private', name: 'G', about: '', picture: '', relay: '', owner: 'o', membership: 'joined',
    my_role: mine, muted: false, can_post: true, history_for_new: true,
    members: [
      ...(mine ? [member(ME, mine)] : []),
      member('owner', 'owner'), member('admin', 'admin'), member('mod', 'moderator'), member('user', 'member'),
    ],
    banned: [], requests: [], request_note: '', link: null, undecrypted: 0, key: null,
    post_mode: 'all', schedule: null, premod: 'all', closed_until: null, needs_update: false, queue: 0, ...over,
  };
}

describe('the moderation a role is offered', () => {
  it('lets a moderator and above hide what someone below wrote, never a peer, a superior or themselves', () => {
    expect(canHide(group('moderator'), 'user')).toBe(true);
    expect(canHide(group('moderator'), 'mod')).toBe(false);
    expect(canHide(group('moderator'), 'admin')).toBe(false);
    expect(canHide(group('admin'), 'mod')).toBe(true);
    expect(canHide(group('owner'), 'admin')).toBe(true);
    expect(canHide(group('owner'), ME)).toBe(false);
    expect(canHide(group('member'), 'user')).toBe(false);
  });

  it('counts a former member, gone from the list, as a member', () => {
    expect(canHide(group('moderator'), 'stranger')).toBe(true);
    expect(canSpam(group('admin'), 'stranger')).toBe(true);
  });

  it('lets moderators and above decide on the queue', () => {
    expect(canApprove(group('moderator'))).toBe(true);
    expect(canApprove(group('owner'))).toBe(true);
    expect(canApprove(group('member'))).toBe(false);
    expect(canApprove(null)).toBe(false);
    expect(isModerator(group('moderator'))).toBe(true);
    expect(isModerator(group('member'))).toBe(false);
  });

  it('offers nothing in a group I am no longer in', () => {
    const left = group('owner', { membership: 'left' });
    expect(canApprove(left)).toBe(false);
    expect(canHide(left, 'user')).toBe(false);
    expect(canSpam(left, 'user')).toBe(false);
  });

  it('keeps spam, a ban, to managers', () => {
    expect(canSpam(group('moderator'), 'user')).toBe(false);
    expect(canSpam(group('admin'), 'user')).toBe(true);
    expect(canSpam(group('admin'), 'mod')).toBe(true);
    expect(canSpam(group('admin'), 'admin')).toBe(false);
  });

  it('gives the requests to join to managers, and a panel to whoever has a tab in it', () => {
    const asks = { requester: 'x', npub: 'npub1x', note: '', created_at: 1, received_at: 1 };
    expect(answersRequests(group('admin'))).toBe(true);
    expect(answersRequests(group('moderator'))).toBe(false);
    expect(answersRequests(group('owner', { kind: 'public' }))).toBe(false);
    expect(answersRequests(group('owner', { kind: 'public', requests: [asks] }))).toBe(true);
    // A core without moderation (no queue): the panel holds the requests alone, for a manager.
    expect(seesModeration(group('admin', { queue: undefined }))).toBe(true);
    expect(seesModeration(group('moderator', { queue: undefined }))).toBe(false);
    expect(seesModeration(group('moderator'))).toBe(true);
    expect(seesModeration(group('member'))).toBe(false);
    expect(seesModeration(group('owner', { membership: 'left' }))).toBe(false);
  });

  it('trusts as it mutes: a moderator on someone below', () => {
    const g = group('moderator');
    expect(canTrust(g, member('user', 'member'))).toBe(true);
    expect(canTrust(g, member('mod', 'moderator'))).toBe(false);
    expect(canTrust(group('member'), member('user', 'member'))).toBe(false);
  });
});

describe('mutedNow', () => {
  it('holds a mute until lifted and lets a timed one end by itself', () => {
    expect(mutedNow(member('u', 'member'), 100)).toBe(false);
    expect(mutedNow(member('u', 'member', { muted: true }), 100)).toBe(true);
    expect(mutedNow(member('u', 'member', { muted: true, muted_until: 200 }), 100)).toBe(true);
    expect(mutedNow(member('u', 'member', { muted: true, muted_until: 200 }), 200)).toBe(false);
  });
});

describe('losesModeration', () => {
  it('is a moderator or above who ends up below one, or out', () => {
    expect(losesModeration('moderator', 'member')).toBe(true);
    expect(losesModeration('moderator', null)).toBe(true);
    expect(losesModeration('admin', 'member')).toBe(true);
    expect(losesModeration('admin', 'moderator')).toBe(false);
    expect(losesModeration('member', null)).toBe(false);
    expect(losesModeration('member', 'moderator')).toBe(false);
  });
});

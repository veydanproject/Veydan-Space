// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/permissions.ts at 8ce09c39, then changed for channels.

// What a role may do in a channel. Mirrors the table the core enforces
// (crates/communities/core/src/roles.rs `permits`, communities-wire.md §3):
// this file only decides what to offer; a refusal still comes back from
// the core when it matters. Roles from the lowest: member (a reader, who
// is not in the journal at all), author, moderator, admin, owner. One acts
// only on roles below one's own, and gives no role equal to or above it.

import type { ChannelDetail, ChannelView, FeedItemView, StaffView } from './api';

export type Role = 'member' | 'author' | 'moderator' | 'admin' | 'owner';

const RANK: Record<Role, number> = { member: 0, author: 1, moderator: 2, admin: 3, owner: 4 };
const AUTHOR = RANK.author;
const MODERATOR = RANK.moderator;
const MANAGER = RANK.admin;

/** The rank of a role; a reader (no role) ranks as a member. */
export const rank = (role: string | null | undefined): number => RANK[(role ?? 'member') as Role] ?? 0;

export const joined = (d: ChannelDetail | null | undefined): d is ChannelDetail => !!d && d.channel.membership === 'joined' && !d.disbanded;

const mine = (d: ChannelDetail | null | undefined): number => (joined(d) ? rank(d.channel.my_role) : -1);

export const isOwner = (d: ChannelDetail | null | undefined): d is ChannelDetail => joined(d) && d.channel.my_role === 'owner';
/** An admin or the owner: the settings, the roles, the bans. */
export const isManager = (d: ChannelDetail | null | undefined): d is ChannelDetail => mine(d) >= MANAGER;
/** A moderator and above: the queue, the log, hiding, mutes, requests, pins, a new key. */
export const isModerator = (d: ChannelDetail | null | undefined): d is ChannelDetail => mine(d) >= MODERATOR;

/** What the journal says of a person: their role, their mute, their trust; a reader it names not, a member. */
export function staffOf(d: ChannelDetail, pubkey: string): StaffView {
  return d.staff.find((s) => s.pubkey === pubkey) ?? { pubkey, role: 'member', muted: false, trusted: false, since: 0 };
}

/** Is the person kept from writing now? A mute for a time ends by itself. */
export const mutedNow = (s: Pick<StaffView, 'muted' | 'muted_until'>, now = Date.now() / 1000): boolean =>
  s.muted && (s.muted_until === null || s.muted_until === undefined || now < s.muted_until);

/** Am I muted in this channel now? */
export const amMuted = (d: ChannelDetail, me: string | null): boolean => !!me && mutedNow(staffOf(d, me));

/**
 * May I post in the feed: an author and above (the team), or a reader whose
 * level lets post into a section (the core says it per section, `post`;
 * 2026-10-23), with a key, and not muted. Other readers comment (the
 * `post_mode` of the wire is `authors` always).
 */
export const canPost = (d: ChannelDetail | null | undefined, me: string | null): boolean =>
  joined(d) && d.has_key && (mine(d) >= AUTHOR || (d.scopes ?? []).some((s) => s.post === true)) && !amMuted(d, me);

/** May I comment: anyone in, while the channel takes comments; with a key, and not muted. */
export const canComment = (d: ChannelDetail | null | undefined, me: string | null): boolean =>
  joined(d) && d.has_key && d.comments === 'on' && !amMuted(d, me);

/** A file needs a place on the hosts: authors and above put blobs there, a reader comments in words. */
export const canAttach = (d: ChannelDetail | null | undefined): boolean => mine(d) >= AUTHOR;

/**
 * A voice message or a circle recorded in the composer (2026-10-23): a post's, as a file goes, by
 * authors and above; a comment's by anyone who may comment (communities-wire §9).
 */
export const canRecord = (d: ChannelDetail | null | undefined, me: string | null, where: 'post' | 'comment'): boolean =>
  where === 'comment' ? canComment(d, me) : canAttach(d);

const below = (d: ChannelDetail, who: string, me: string | null): boolean => who !== me && rank(staffOf(d, who).role) < mine(d);

/** Mute, trust, remove, ban, hide what they wrote: a moderator on a role below. */
export const canModerate = (d: ChannelDetail | null | undefined, who: string, me: string | null): boolean =>
  isModerator(d) && below(d, who, me);
export const canMute = canModerate;
export const canTrust = canModerate;
export const canRemove = canModerate;
export const canBan = canModerate;
/**
 * Giving levels (`set_levels`, communities-wire §14.1): a moderator, to a
 * reader or an author below them. The team (moderator and above) is in
 * every section already.
 */
export const canSetLevels = (d: ChannelDetail | null | undefined, who: string, me: string | null): boolean =>
  canModerate(d, who, me) && rank(staffOf(d as ChannelDetail, who).role) < MODERATOR;
/** Lifting a ban, letting people in, pinning: a moderator's. */
export const canUnban = (d: ChannelDetail | null | undefined): boolean => isModerator(d);
export const answersRequests = (d: ChannelDetail | null | undefined): d is ChannelDetail => isModerator(d);
export const canPin = (d: ChannelDetail | null | undefined): boolean => isModerator(d);
export const canRotateLink = (d: ChannelDetail | null | undefined): boolean => isModerator(d);
export const canRotateKey = (d: ChannelDetail | null | undefined): boolean => isModerator(d);
export const canInvite = (d: ChannelDetail | null | undefined): boolean => isModerator(d);

/** Roles I may give this person: a manager, below their own; an admin is made by the owner alone. */
export function rolesFor(d: ChannelDetail, who: string, me: string | null): Role[] {
  if (!isManager(d) || !below(d, who, me)) return [];
  const now = staffOf(d, who).role;
  return (['member', 'author', 'moderator', 'admin'] as Role[]).filter(
    (r) => r !== now && rank(r) < mine(d) && (r !== 'admin' || isOwner(d)),
  );
}

/** May I remove this post or comment: my own, or one of a role below mine as a moderator. */
export const canDelete = (d: ChannelDetail | null | undefined, item: Pick<FeedItemView, 'author'>, me: string | null): boolean =>
  !!d && (item.author === me || canModerate(d, item.author, me));

/**
 * Only the author edits: the words of a post or a comment, the caption of
 * a post with a file (of an album, the last post, which carries it; the
 * core edits a caption since stage 1 of the redesign, 2026-10-23). The
 * files themselves stay as they went.
 */
export const canEdit = (item: Pick<FeedItemView, 'author' | 'deleted'>, me: string | null): boolean =>
  !!me && item.author === me && !item.deleted;

/** Does the channel check messages before they are shown? */
export const checksMessages = (d: ChannelDetail | null | undefined): boolean => !!d && d.premod !== 'off';

/**
 * A moderator's decisions stand while the moderator does: a change that
 * leaves them below a moderator (or out) is the moment to offer keeping
 * them under my name.
 */
export function losesModeration(before: string, after: string | null): boolean {
  return rank(before) >= MODERATOR && (after === null || rank(after) < MODERATOR);
}

/**
 * Where a member of the channel stands, as the page shows it: in and
 * reading, in and waiting for the keeper's key, in with an old key and
 * catching up, or one of the ways of being outside.
 */
/** How long a key may be awaited, and the keeper unheard of, before the page says the keeper is away. */
export const KEEPER_AWAY_SECS = 10 * 60;

/**
 * The keeper is away for a channel whose key I wait for: I have waited
 * `KEEPER_AWAY_SECS` (since the row last changed, my join) and no key of
 * the keeper's came in as long (`keeper_last_seen`: the newest epoch or
 * sign of life of the keeper every reader sees, wire §18), or none ever did.
 */
export function keeperAwayAt(c: Pick<ChannelView, 'updated_at' | 'keeper_last_seen'>, now: number): boolean {
  if (now - c.updated_at < KEEPER_AWAY_SECS) return false;
  return c.keeper_last_seen === undefined || now - c.keeper_last_seen >= KEEPER_AWAY_SECS;
}

export type Standing = 'reading' | 'waiting_key' | 'catching_up' | 'joining' | 'requested' | 'rejected' | 'out' | 'none' | 'disbanded';

export function standing(d: ChannelDetail): Standing {
  if (d.disbanded || d.channel.membership === 'disbanded') return 'disbanded';
  switch (d.channel.membership) {
    case 'joined':
      if (!d.has_key) return 'waiting_key';
      return d.waiting > 0 ? 'catching_up' : 'reading';
    case 'joining':
      return 'joining';
    case 'requested':
      return 'requested';
    case 'rejected':
      return 'rejected';
    case 'out':
      return 'out';
    default:
      return 'none';
  }
}

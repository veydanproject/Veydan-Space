// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What a role may do. Mirrors the table the messenger core enforces
// (internal/messenger-spec.md, stage 8): this file only decides what to
// offer; a refusal still comes back from the core when it matters.

import { roleRank, type GroupRole, type MessengerGroup, type MessengerGroupMember } from '../api';

const MANAGER = 2;
const MODERATOR = 1;

export const isManager = (g: MessengerGroup): boolean => g.membership === 'joined' && roleRank(g.my_role) >= MANAGER;
export const isOwner = (g: MessengerGroup): boolean => g.membership === 'joined' && g.my_role === 'owner';

const below = (g: MessengerGroup, m: MessengerGroupMember): boolean => !m.is_me && roleRank(m.role) < roleRank(g.my_role);

export const canRemove = (g: MessengerGroup, m: MessengerGroupMember): boolean => isManager(g) && below(g, m);
export const canMute = (g: MessengerGroup, m: MessengerGroupMember): boolean =>
  g.membership === 'joined' && roleRank(g.my_role) >= MODERATOR && below(g, m);

/** Roles I may give to this member. */
export function rolesFor(g: MessengerGroup, m: MessengerGroupMember): GroupRole[] {
  if (!isManager(g) || !below(g, m)) return [];
  return (['member', 'moderator', 'admin'] as GroupRole[]).filter(
    (r) => r !== m.role && roleRank(r) < roleRank(g.my_role),
  );
}

/** May I remove for everyone a message written by this person? */
export function canModerate(g: MessengerGroup | null, author: string): boolean {
  if (!g || g.membership !== 'joined' || roleRank(g.my_role) < MODERATOR) return false;
  const role = g.members.find((m) => m.pubkey === author)?.role ?? 'member';
  return roleRank(role) < roleRank(g.my_role);
}

/** A moderator or above in a group I am in: the queue, the log, hiding. */
export const isModerator = (g: MessengerGroup | null): g is MessengerGroup =>
  !!g && g.membership === 'joined' && roleRank(g.my_role) >= MODERATOR;

/**
 * Does the core behind the page moderate groups (the queue, the marks,
 * timed mutes, who may write and when)? A core without it sends none of
 * those fields and answers their commands with `not_implemented`; the
 * page then keeps to the operations of the log it has always had.
 */
export const hasModeration = (g: MessengerGroup | null): g is MessengerGroup => !!g && g.queue !== undefined;

/** Does the group check messages before they are shown? Read defensively: a core without moderation says nothing. */
export const checksMessages = (g: MessengerGroup | null): boolean => !!g?.premod && g.premod !== 'off';

/**
 * May I hide what this person wrote? A moderator hides, never deletes for
 * everyone (the owner's decision 8); the same rule as `canModerate`.
 */
export const canHide = (g: MessengerGroup | null, author: string): boolean => canModerate(g, author);

/** May I let through, or turn down, what waits for a moderator? */
export const canApprove = (g: MessengerGroup | null): boolean => isModerator(g);

/** Spam is a ban and a hiding: a manager's, on someone below them. */
export function canSpam(g: MessengerGroup | null, author: string): boolean {
  if (!g || !isManager(g)) return false;
  const role = g.members.find((m) => m.pubkey === author)?.role ?? 'member';
  return roleRank(role) < roleRank(g.my_role);
}

/** Letting someone post without a check, or checking them again: as muting. */
export const canTrust = (g: MessengerGroup, m: MessengerGroupMember): boolean => canMute(g, m);

/** Is the member kept from writing now? A mute for a time ends by itself. */
export const mutedNow = (m: MessengerGroupMember, now = Date.now() / 1000): boolean =>
  m.muted && (m.muted_until === null || m.muted_until === undefined || now < m.muted_until);

/**
 * A moderator's decisions stand while the moderator does: a change that
 * leaves them below a moderator (or out) is the moment to offer keeping
 * them under my name.
 */
export function losesModeration(before: GroupRole, after: GroupRole | null): boolean {
  return roleRank(before) >= MODERATOR && roleRank(after) < MODERATOR;
}

/**
 * The network closed the group (a confirmed case of harm to a child). The
 * group's own view already carries a `banned` list of members, so the flag
 * is read from the chat row alone; absent from an older runtime, so false.
 */
export const isBannedGroup = (chat: { banned?: boolean } | null | undefined): boolean => !!chat?.banned;

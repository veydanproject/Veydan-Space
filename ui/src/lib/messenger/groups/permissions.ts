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

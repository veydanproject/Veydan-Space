// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What happened to a group, as a line in its chat.

import type { MessengerMessage } from '../api';

type Translate = (key: string, params?: Record<string, string>) => string;

const KNOWN = new Set([
  'group_created', 'group_admitted', 'group_joined', 'group_left', 'group_removed', 'group_banned', 'group_unbanned',
  'group_role', 'group_muted', 'group_unmuted', 'group_edited', 'group_owner', 'group_link_changed', 'group_disbanded',
]);

/** `null` when the line is not about a group. */
export function groupLine(m: MessengerMessage, tr: Translate, name: (pubkey: string) => string): string | null {
  if (m.content_type !== 'system' || !m.text || !KNOWN.has(m.text)) return null;
  const d = (m.media ?? {}) as { actor?: string; target?: string | null; role?: string | null };
  const actor = d.actor ? name(d.actor) : '';
  const target = d.target ? name(d.target) : '';
  const role = d.role ? tr(`msg_group_role_${d.role}`) : '';
  return tr(`msg_sys_${m.text}`, { actor, target, role });
}

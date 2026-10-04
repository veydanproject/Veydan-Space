// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Refusals of the group module arrive as stable codes (`group_…`).

import { groupErrorCode, messengerError } from '../api';

type Translate = (key: string, params?: Record<string, string>) => string;

const KNOWN = new Set([
  'group_not_permitted', 'group_not_member', 'group_muted', 'group_unknown', 'group_already_member', 'group_target_banned',
  'group_author_banned', 'group_stale_link', 'group_no_key', 'group_no_relay', 'group_full', 'group_leave_first',
  'group_invite_expired', 'group_invite_answered', 'group_invite_unknown', 'group_request_unknown', 'group_disbanded',
  'group_target_not_member', 'group_invalid', 'group_wrong_kind',
]);

/** Text for a refusal of the group module; `null` when the error is not one. */
export function groupRefusal(e: unknown, tr: Translate): string | null {
  const code = groupErrorCode(e);
  if (!code) return null;
  return tr(KNOWN.has(code) ? `msg_err_${code}` : 'msg_err_group_other', { code });
}

export function groupError(e: unknown, tr: Translate): string {
  return groupRefusal(e, tr) ?? messengerError(e);
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/errors.ts at 8ce09c39, then changed for channels.

// Refusals of the module arrive as `channel_<code>: text` (the message of
// the command's error, crates/communities `code_of`): a known code is
// worded, any other is said as «something went wrong» with its code.

import { channelErrorCode, errorCode, errorText } from './api';

type Translate = (key: string, params?: Record<string, string>) => string;

/** The codes worded by the dictionary (`chan_err_<code>`, without `channel_`). */
export const KNOWN = new Set([
  'unknown', 'unknown_message', 'not_permitted', 'upload_not_permitted', 'no_keeper', 'no_key', 'not_here_yet', 'link_owner',
  'owner_stays', 'kept_here', 'too_many_invites', 'already_member', 'invite_only', 'invite_expired', 'invite_unknown', 'not_author', 'banned',
  'upload_removed', 'file_unsupported', 'self', 'no_relay', 'invalid', 'bad_link', 'bad_key', 'not_delivered', 'transport',
  'preview_unreachable', 'upload_cancelled', 'upload_refused', 'cover_unsupported', 'cover_too_large', 'cover_address',
  // The channel's own server takes files from its team alone (communities-wire §9.1).
  'storage_needs_key',
  // Levels and sections (communities-wire §14).
  'not_member_of_scope', 'scope_no_post', 'no_transfer', 'tickets_open', 'level_name', 'level_last', 'level_limit', 'scope_everyone', 'scope_limit',
]);

/** Text for a refusal of the module; `null` when the error is not one. */
export function channelRefusal(e: unknown, tr: Translate): string | null {
  const code = channelErrorCode(e);
  if (!code) return null;
  const short = code.slice('channel_'.length);
  // A host that refused a file: what it answered (its address, its words), after the code.
  const answer = errorText(e).trim().replace(/^channel_[a-z_]+:\s*(channel: upload refused:\s*)?/, '');
  return tr(KNOWN.has(short) ? `chan_err_${short}` : 'chan_err_other', { code, answer });
}

/** Text for any error of a command of the module. */
export function channelError(e: unknown, tr: Translate): string {
  // The lock closed under the page: the keys of the channels are in its box.
  if (errorCode(e) === 'vault_locked') return tr('chan_err_locked');
  return channelRefusal(e, tr) ?? errorText(e);
}

/** A refusal of the moderation: the same codes, said the same way. */
export const moderationError = channelError;

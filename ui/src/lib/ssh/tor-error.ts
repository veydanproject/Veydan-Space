// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { TranslationKey } from '$lib/core/i18n';
import { formatError } from '$lib/core/utils';

/** What the backend puts before the English text of an error that came through Tor: `tor_failed: …`. */
const KEYS = {
  tor_not_installed: 'ssh_tor_err_not_installed',
  tor_invalid_country: 'ssh_tor_err_invalid_country',
  tor_too_many_instances: 'ssh_tor_err_too_many_instances',
  tor_bootstrap_timeout: 'ssh_tor_err_bootstrap_timeout',
  tor_failed: 'ssh_tor_err_failed',
  tor_stopped: 'ssh_tor_err_stopped',
  tor_in_use: 'ssh_tor_err_in_use',
  tor_no_builtin_bridges: 'ssh_tor_err_no_builtin_bridges',
  tor_no_transports: 'ssh_tor_err_no_transports',
} as const satisfies Record<string, TranslationKey>;

export type TorErrorCode = keyof typeof KEYS;

/** The first known `tor_*` code followed by `: ` in an error message, or null. */
export function torErrorCode(message: string | null | undefined): TorErrorCode | null {
  if (!message) return null;
  for (const m of message.matchAll(/(?<![a-z_])(tor_[a-z_]+): /g)) {
    if (m[1] in KEYS) return m[1] as TorErrorCode;
  }
  return null;
}

/** The translated, actionable text for a message with a Tor code; any other message as it is. */
export function explainTorMessage(message: string, tr: (key: TranslationKey) => string): string {
  const code = torErrorCode(message);
  return code ? tr(KEYS[code]) : message;
}

/** `formatError` that also turns the Tor codes into texts that say what to do. */
export function explainError(e: unknown, tr: (key: TranslationKey) => string): string {
  return explainTorMessage(formatError(e), tr);
}

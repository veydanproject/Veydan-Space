// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { hasErrorCode } from '$lib/core/utils';
import type { TranslationKey } from '$lib/core/i18n';

/** User-facing text for a password-vault error, or null when it is not one. */
export function passwordErrorKey(e: unknown): TranslationKey | null {
  if (hasErrorCode(e, 'vault_locked')) return 'pw_err_locked';
  if (hasErrorCode(e, 'vault_mismatch')) return 'pw_err_mismatch';
  if (hasErrorCode(e, 'decrypt_failed')) return 'pw_err_decrypt';
  if (hasErrorCode(e, 'recovery_invalid')) return 'lock_recovery_invalid';
  return null;
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The words the Recovery Key is refused with (cloud-recovery.md §6), the
// same on the page «Account» and in the step of the first start.

import type { TranslationKey } from '$lib/core/i18n';
import type { CloudError } from '$lib/cloud/api';

/** The words of a refusal of the module, by its code (cloud-api.md §12). */
export function recoveryRefusal(e: CloudError, tr: (key: TranslationKey) => string): string {
  switch (e.code) {
    case 'wrong_password':
      return tr('cloud_recovery_wrong_password');
    case 'already_set_up':
      return tr('cloud_recovery_already');
    case 'issued_elsewhere':
      return tr('cloud_recovery_elsewhere');
    case 'reissue_pending':
      return tr('cloud_recovery_pending');
    case 'no_sync_vault':
      return tr('cloud_recovery_no_vault');
    default:
      return e.message ? `${e.code}: ${e.message}` : e.code;
  }
}

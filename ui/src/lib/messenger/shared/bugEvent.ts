// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A bug report that waited for a connection (`shared/bug.ts`): what became
// of it, said once by a toast wherever the user is.

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { toast } from '$lib/core/ui/toast.svelte';
import type { MessengerUiEvent } from '../api';

export function bugEvent(event: MessengerUiEvent) {
  if (event.name !== 'bug.report') return;
  const status = (event.payload as { status?: string } | null)?.status;
  const tr = get(t);
  if (status === 'sent') toast.success(tr('msg_bug_sent_later'));
  else if (status === 'refused') toast.error(tr('msg_bug_refused'));
  else if (status === 'expired') toast.error(tr('msg_bug_expired'));
}

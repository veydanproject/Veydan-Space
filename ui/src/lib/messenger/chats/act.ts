// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// An action over a chat (pin, mute, mark read, archive), from the list of chats
// or from the menu of the open chat: a failure is one error toast that says
// which action failed, with «Повторить».

import { get } from 'svelte/store';
import { t, type TranslationKey } from '$lib/core/i18n';
import { toast } from '$lib/core/ui/toast.svelte';

export function act(failed: TranslationKey, run: () => Promise<unknown>): void {
  run().catch(() => toast.error((get(t) as (k: string) => string)(failed), { onAction: () => act(failed, run) }));
}

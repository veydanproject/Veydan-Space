// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Back to the room of the group call I am in: the phone's page of it, or
// the computer's window unfolded from its capsule.

import { goto } from '$app/navigation';
import { groupCallHref } from '../mobile/routes';
import { groupCallStore } from './groupCallStore.svelte';

export function openGroupRoom(phone: boolean): void {
  // On a phone the watcher may have opened it already, at the first word of the room.
  if (!phone) groupCallStore.shown = true;
  else if (location.pathname !== groupCallHref()) void goto(groupCallHref());
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The phone's screens of the messenger (mobile/) mount the same chat, info
// and settings pieces as the desktop page. They mark themselves, and a piece
// lays itself out for a phone where it matters: a back chevron instead of a
// close cross, no keyboard raised on its own, the header in the bar colour.
// The module knows nothing of the platform; its phone screens do.

import { getContext, setContext } from 'svelte';

const KEY = Symbol('messenger.phone');

/** Called while a phone screen of the messenger is set up. */
export function markPhone(): void {
  setContext(KEY, true);
}

/** Inside a phone screen of the messenger (read while a component is set up). */
export function onPhone(): boolean {
  return getContext<boolean | undefined>(KEY) === true;
}

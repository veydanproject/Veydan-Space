// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The cover of a channel for the messenger's Avatar. Of a channel I am in
// (`communityId`) the module reads the picture from the channel's own
// hosts (`coverStore`); a picture already here (a `data:` URL) is shown
// as it is (`src`). The card of a channel not joined yet has no hosts of
// its own here: its address is left to the runtime (`url`), which shows
// initials where it does not fetch it.

import { coverStore } from './coverStore.svelte';

export function cover(picture: string | null | undefined, communityId?: string): { url: string | null; src: string | null } {
  if (!picture) return { url: null, src: null };
  if (picture.startsWith('data:')) return { url: null, src: picture };
  if (communityId) return { url: null, src: coverStore.src(communityId, picture) };
  return { url: picture, src: null };
}

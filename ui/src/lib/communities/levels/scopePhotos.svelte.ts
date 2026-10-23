// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The photos of the sections (communities-wire §14.7) as `data:` URLs, as
// the covers of the channels are kept (coverStore): the module reads the
// picture from the channel's hosts (`channel_scope_photo`), keyed by the
// photo's id, so a new photo is read once and the head, the tiles and the
// editor share the answer. A failure is asked again after a while.

import { channelsApi } from '../api';

const RETRY_MS = 60_000;

class ScopePhotos {
  private data = $state<Record<string, string | null>>({});
  private asked = new Map<string, number>();

  /** The picture of the photo `photo` of a section; `null` while it is on its way (the page shows the icon). */
  src(communityId: string, scopeId: string, photo: string | null | undefined): string | null {
    if (!photo) return null;
    const key = `${communityId}\n${scopeId}\n${photo}`;
    const have = this.data[key];
    if (have) return have;
    const at = this.asked.get(key);
    if (at === undefined || (have === null && Date.now() - at > RETRY_MS)) {
      this.asked.set(key, Date.now());
      // Out of the render that read it: state is written after it.
      queueMicrotask(() => void this.load(communityId, scopeId, key));
    }
    return null;
  }

  private async load(communityId: string, scopeId: string, key: string) {
    let got: string | null = null;
    try {
      got = await channelsApi.levels.scopePhoto(communityId, scopeId);
    } catch {
      got = null;
    }
    if (got) {
      const of = `${communityId}\n${scopeId}\n`;
      for (const k of Object.keys(this.data)) if (k !== key && k.startsWith(of)) delete this.data[k];
    }
    this.data[key] = got;
  }
}

export const scopePhotos = new ScopePhotos();

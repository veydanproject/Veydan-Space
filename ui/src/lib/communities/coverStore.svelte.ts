// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The covers of the channels I am in, as `data:` URLs. The messenger's
// avatar cache fetches only the faces of people, so an address of a cover
// would stay initials; the module reads the picture from the hosts of its
// channel (`channel_cover`) and keeps it on disk by its hash. Each cover
// is asked once per session and address: the list of chats, the card and
// the header of a feed read the same answer. A failure is asked again
// after a while, by the next render that needs the cover.

import { channelsApi } from './api';

const RETRY_MS = 60_000;

class CoverStore {
  /** The cover by channel and address: a `data:` URL, `null` when it could not be read. */
  private data = $state<Record<string, string | null>>({});
  private asked = new Map<string, number>();

  /**
   * The picture to show for the cover `picture` of channel `communityId`:
   * a `data:` one as it is, the module's copy of an address once read,
   * `null` while it is on its way (the page shows initials).
   */
  src(communityId: string, picture: string | null | undefined): string | null {
    if (!picture) return null;
    if (picture.startsWith('data:')) return picture;
    const key = `${communityId}\n${picture}`;
    const have = this.data[key];
    if (have) return have;
    const at = this.asked.get(key);
    if (at === undefined || (have === null && Date.now() - at > RETRY_MS)) {
      this.asked.set(key, Date.now());
      // Out of the render that read it: state is written after it.
      queueMicrotask(() => void this.load(communityId, key));
    }
    return null;
  }

  private async load(communityId: string, key: string) {
    let got: string | null = null;
    try {
      got = await channelsApi.cover(communityId);
    } catch {
      got = null;
    }
    if (got) {
      // The older covers of this channel are no longer shown.
      for (const k of Object.keys(this.data)) if (k !== key && k.startsWith(`${communityId}\n`)) delete this.data[k];
    }
    this.data[key] = got;
  }
}

export const coverStore = new CoverStore();

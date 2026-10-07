// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Pictures of people as the page may show them. The WebView never loads an
// image from the network: the runtime fetches a picture (SSRF-safe, size
// limits), re-encodes it and keeps it on disk; the page gets a `data:` URL.
// Each URL is asked once per session; a picture still on its way comes
// with `avatar.ready {url}`, and the URL is asked again then.

import { messengerApi, type MessengerUiEvent } from '../api';

/** A `data:` URL, `pending` while the runtime fetches, `null` when there is none. */
export type AvatarState = string | 'pending' | null;

/** What the runtime gives: a re-encoded raster picture, nothing else. */
const DATA_IMAGE = /^data:image\/(?:jpeg|png|webp|gif);base64,[A-Za-z0-9+/=]+$/;

/** Only a picture the runtime made may be an `<img src>`. */
export function safeDataImage(v: string | null | undefined): string | null {
  return v && DATA_IMAGE.test(v) ? v : null;
}

class AvatarStore {
  private states = $state<Record<string, AvatarState>>({});
  /** Asked this session; not reactive: asking must not wake a render. */
  private asked = new Set<string>();

  /**
   * The picture at `url` as a `data:` URL, or `null` until there is one.
   * Reading it in a template subscribes; the first read asks the runtime.
   */
  get(url: string | null | undefined): string | null {
    const v = this.state(url);
    return v && v !== 'pending' ? v : null;
  }

  /** The state of `url`; `undefined` for an address that is never asked (not https). */
  state(url: string | null | undefined): AvatarState | undefined {
    if (!url || !url.startsWith('https://')) return undefined;
    const v = this.states[url];
    if (!this.asked.has(url)) {
      this.asked.add(url);
      // Out of the render that read it: state is written after it.
      queueMicrotask(() => this.load(url));
    }
    return v === undefined ? 'pending' : v;
  }

  /** Runtime event → state. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    if (ev.name !== 'avatar.ready') return;
    const url = (ev.payload as { url?: string } | null)?.url;
    if (typeof url === 'string' && url.startsWith('https://')) {
      this.asked.add(url);
      this.load(url);
    }
  }

  /** The module stopped: nothing is kept. */
  reset() {
    this.states = {};
    this.asked.clear();
  }

  private async load(url: string) {
    try {
      // `null` means it is on its way (or failed lately): `avatar.ready` brings it.
      const data = safeDataImage(await messengerApi.avatar.cached(url));
      if (data || this.states[url] === undefined) this.states[url] = data ?? 'pending';
    } catch {
      // Refused (not an address it fetches): initials for the session.
      this.states[url] = null;
    }
  }
}

export const avatarStore = new AvatarStore();

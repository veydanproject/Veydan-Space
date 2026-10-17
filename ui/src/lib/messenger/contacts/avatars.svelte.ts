// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Pictures of people as the page may show them. The WebView never loads an
// image from the network: the runtime fetches a picture (SSRF-safe, size
// limits), re-encodes it and keeps it on disk; the page gets a `data:` URL.
// Each URL is asked once per session; a picture still on its way comes
// with `avatar.ready {url}`, and the URL is asked again then.
//
// The runtime fetches only the faces it has a reason to (a contact, a
// member of my group, a vouched invite), and an address that failed waits
// before it is fetched again; either way it answers "not here" and sends
// no `avatar.ready`. So an address still waiting is asked again when the
// reason may have changed (a contact made, a request accepted, a group
// joined), and every few minutes while a face shows it: the runtime
// answers from its cache, and fetches when its own rules allow.

import { messengerApi, type MessengerUiEvent } from '../api';

/** A `data:` URL, `pending` while the runtime fetches, `null` when there is none. */
export type AvatarState = string | 'pending' | null;

/** A face on the page still waiting for its picture is asked again this often. */
export const AVATAR_RETRY_MS = 3 * 60_000;
/** Events come in bursts (a sync applies many): one round of asking after them. */
export const AVATAR_SETTLE_MS = 1_000;

/**
 * Events after which a face the runtime did not fetch may be one it does:
 * the book, a relation (a request accepted, another device's decision), a
 * profile, a group and its members, an invite.
 */
const REASONS = new Set([
  'contacts.updated',
  'follows.updated',
  'profile.updated',
  'dm.relationship',
  'dm.relations',
  'group.updated',
  'group.invite',
  'history.synced',
]);

/** What the runtime gives: a re-encoded raster picture, nothing else. */
const DATA_IMAGE = /^data:image\/(?:jpeg|png|webp|gif);base64,[A-Za-z0-9+/=]+$/;

/** Only a picture the runtime made may be an `<img src>`. */
export function safeDataImage(v: string | null | undefined): string | null {
  return v && DATA_IMAGE.test(v) ? v : null;
}

const waiting = (v: AvatarState | undefined) => v === 'pending' || v === null;

class AvatarStore {
  private states = $state<Record<string, AvatarState>>({});
  /** Asked this session; not reactive: asking must not wake a render. */
  private asked = new Set<string>();
  /** How many faces on the page show each address (`show`); not reactive. */
  private shown = new Map<string, number>();
  /** Addresses being asked again: a round does not ask twice at once. */
  private again = new Set<string>();
  private settleTimer: ReturnType<typeof setTimeout> | null = null;
  private retryTimer: ReturnType<typeof setInterval> | null = null;

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

  /**
   * A face shows `url` until the returned function is called: while it
   * waits, it is asked again now and then. Called by `Avatar.svelte`.
   */
  show(url: string | null | undefined): () => void {
    if (!url || !url.startsWith('https://')) return () => {};
    this.shown.set(url, (this.shown.get(url) ?? 0) + 1);
    return () => {
      const n = (this.shown.get(url) ?? 1) - 1;
      if (n > 0) this.shown.set(url, n);
      else this.shown.delete(url);
    };
  }

  /** Runtime event → state. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    if (ev.name === 'avatar.ready') {
      const url = (ev.payload as { url?: string } | null)?.url;
      if (typeof url === 'string' && url.startsWith('https://')) {
        this.asked.add(url);
        this.load(url);
      }
      return;
    }
    if (REASONS.has(ev.name) && !this.settleTimer) {
      this.settleTimer = setTimeout(() => {
        this.settleTimer = null;
        this.askAgain();
      }, AVATAR_SETTLE_MS);
    }
  }

  /** The module stopped: nothing is kept. */
  reset() {
    this.states = {};
    this.asked.clear();
    this.again.clear();
    if (this.settleTimer) clearTimeout(this.settleTimer);
    this.settleTimer = null;
    if (this.retryTimer) clearInterval(this.retryTimer);
    this.retryTimer = null;
  }

  /**
   * Every address still waiting is asked again: at once when a face shows
   * it, at its next read otherwise. With none waiting, the rounds stop.
   */
  private askAgain() {
    const urls = Object.keys(this.states).filter((url) => waiting(this.states[url]));
    if (!urls.length && this.retryTimer) {
      clearInterval(this.retryTimer);
      this.retryTimer = null;
    }
    for (const url of urls) {
      if (!this.shown.has(url)) {
        this.asked.delete(url);
        continue;
      }
      if (this.again.has(url)) continue;
      this.again.add(url);
      this.load(url).finally(() => this.again.delete(url));
    }
  }

  private keepAsking() {
    if (!this.retryTimer) this.retryTimer = setInterval(() => this.askAgain(), AVATAR_RETRY_MS);
  }

  private async load(url: string) {
    try {
      // `null` means it is on its way (or failed lately): `avatar.ready`
      // brings it, or a later round asks again. A picture shown stays.
      const data = safeDataImage(await messengerApi.avatar.cached(url));
      if (data) this.states[url] = data;
      else if (waiting(this.states[url]) || this.states[url] === undefined) {
        this.states[url] = 'pending';
        this.keepAsking();
      }
    } catch {
      // Refused (locked, or not an address it fetches): initials, and
      // asked again with the others that wait.
      if (waiting(this.states[url]) || this.states[url] === undefined) {
        this.states[url] = null;
        this.keepAsking();
      }
    }
  }
}

export const avatarStore = new AvatarStore();

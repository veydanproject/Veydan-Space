// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The files of the channels fetched while the app runs, by the hash of the
// file: where it is on the device, and the `data:` URL a page shows it from.
// A post drawn again (scrolled away and back, a channel opened again, the
// same picture in the feed and in a thread) shows its picture at once and
// asks the runtime nothing; two asking at once share one fetch. The URLs
// are kept up to a number of bytes, the least recently shown going first.

import type { FeedItemView, PostMedia } from './api';

export interface MediaEntry {
  /** Where the file is on this device. */
  path: string;
  /** The file to show in place; `null` when a page cannot show it (too large, a type it does not show). */
  src: string | null;
  /** The URL was too large to keep: it is fetched again when shown. */
  large?: boolean;
}

/** The bytes of URLs kept: a few dozen pictures of a feed. */
export const MAX_BYTES = 48 * 1024 * 1024;
/** One URL larger than this is not kept: it would push out all the rest. */
export const MAX_ONE = MAX_BYTES / 4;

/** A file by its hash (the same file in two posts is one), else by its message. */
export function mediaKey(item: Pick<FeedItemView, 'community_id' | 'id'>, m: Pick<PostMedia, 'sha256'> | null): string {
  return m?.sha256 ? `sha:${m.sha256}` : `id:${item.community_id}:${item.id}`;
}

export class MediaCache {
  /** In the order of use, the most recent last. */
  private entries = new Map<string, MediaEntry>();
  private flights = new Map<string, Promise<MediaEntry>>();
  private used = 0;

  constructor(private max = MAX_BYTES, private maxOne = MAX_ONE) {}

  get bytes(): number {
    return this.used;
  }

  /** What is kept of a file, made the most recent. */
  get(key: string): MediaEntry | undefined {
    const e = this.entries.get(key);
    if (e) {
      this.entries.delete(key);
      this.entries.set(key, e);
    }
    return e;
  }

  put(key: string, entry: MediaEntry) {
    const old = this.entries.get(key);
    if (old) {
      this.used -= old.src?.length ?? 0;
      this.entries.delete(key);
    }
    // Too large to keep in memory: its place on the device is kept, the page fetches the URL again.
    const kept = entry.src && entry.src.length > this.maxOne ? { ...entry, src: null, large: true } : entry;
    this.entries.set(key, kept);
    this.used += kept.src?.length ?? 0;
    for (const [k, e] of this.entries) {
      if (this.used <= this.max) break;
      if (k === key) continue;
      this.used -= e.src?.length ?? 0;
      this.entries.delete(k);
    }
  }

  /** The file, kept or fetched once for everyone who asks meanwhile; a failure is not kept. */
  fetch(key: string, load: () => Promise<MediaEntry>): Promise<MediaEntry> {
    const kept = this.get(key);
    if (kept && !kept.large) return Promise.resolve(kept);
    let flight = this.flights.get(key);
    if (!flight) {
      flight = load()
        .then((e) => { this.put(key, e); return e; })
        .finally(() => this.flights.delete(key));
      this.flights.set(key, flight);
    }
    return flight;
  }

  clear() {
    this.entries.clear();
    this.flights.clear();
    this.used = 0;
  }
}

export const mediaCache = new MediaCache();

/**
 * Calls `onnear` once the element comes within `margin` of the viewport
 * (a picture is fetched a little before it is scrolled to, not all of a
 * feed at once); at once where there is no IntersectionObserver.
 */
export function nearView(node: Element, opts: { onnear: () => void; margin?: string }) {
  if (typeof IntersectionObserver === 'undefined') {
    opts.onnear();
    return {};
  }
  const io = new IntersectionObserver((entries) => {
    if (entries.some((e) => e.isIntersecting)) {
      io.disconnect();
      opts.onnear();
    }
  }, { rootMargin: opts.margin ?? '800px 0px' });
  io.observe(node);
  return { destroy: () => io.disconnect() };
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/content/timeline.ts at 30def66d (mediaFamily, albumKey, the gathering of an album), then changed for channels.

// Files picked together and sent as one (`batch` of their descriptors, as
// the messenger's albums) are drawn as one post: consecutive posts of one
// author with the same batch on one day, up to ten; pictures and videos as
// a mosaic, other files as a list. The last of them carries the caption,
// the reactions and the comments of the whole.

import { mediaOf, type FeedItemView } from './api';
import { isLocal } from './feedStore.svelte';

/** As many as one album holds; more sent at once make several. */
export const MAX_ALBUM = 10;

/** What a file is shown as in an album: a picture, or a file card; `null` for anything never put in one. */
export function mediaFamily(item: Pick<FeedItemView, 'media_json' | 'deleted'>): 'visual' | 'files' | null {
  if (item.deleted) return null;
  const m = mediaOf(item);
  if (!m) return null;
  if (m.kind === 'image' || m.kind === 'video') return 'visual';
  if (m.kind === 'file' || m.kind === 'audio') return 'files';
  return null;
}

/** What says that posts were sent by one action; `null` when nothing does. */
export function albumKey(item: Pick<FeedItemView, 'media_json' | 'deleted'>): string | null {
  return mediaFamily(item) ? (mediaOf(item)?.batch ?? null) : null;
}

/** One post as drawn: a post alone, or the posts of one album, oldest first. */
export interface FeedUnit {
  /** The id of its first post: the album keeps its place as its parts come. */
  key: string;
  items: FeedItemView[];
}

const dayOf = (at: number) => new Date(at * 1000).toDateString();

/** The feed as drawn: consecutive posts of one album gathered into one unit. */
export function feedUnits(items: readonly FeedItemView[]): FeedUnit[] {
  const out: FeedUnit[] = [];
  for (const x of items) {
    const prev = out[out.length - 1];
    const last = prev?.items[prev.items.length - 1];
    const key = isLocal(x.id) ? null : albumKey(x);
    if (key && prev && last && albumKey(last) === key && last.author === x.author && prev.items.length < MAX_ALBUM
      && dayOf(last.created_at) === dayOf(x.created_at)) {
      prev.items.push(x);
      continue;
    }
    out.push({ key: x.id, items: [x] });
  }
  return out;
}

/** How the parts of an album are drawn: pictures and videos, then the other files. */
export function albumParts(items: readonly FeedItemView[]): { visual: FeedItemView[]; files: FeedItemView[] } {
  return {
    visual: items.filter((x) => mediaFamily(x) === 'visual'),
    files: items.filter((x) => mediaFamily(x) !== 'visual'),
  };
}

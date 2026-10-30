// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the grid of the tab «Галерея» holds, apart from how it is drawn:
// the pages of the newest photos and videos as the source gives them, the
// choice in the order it was made, the thumbnails asked a few at a time for
// the tiles on the screen, and which rows of the grid are on the screen.
// A source of 10 000 photos costs the pages scrolled through and the tiles
// in view: the rest is never drawn nor asked for. When the app comes back
// to the screen everything is read anew (`reread`): the user may have
// changed the access, or only the set a partial access lets in. What is
// chosen and what is viewed never depend on the album shown nor on the
// pages read: each chosen item is asked of the source by itself.

import type { MediaAccess, MediaItem, MediaKind, MediaLibrary } from './MediaLibrary';

/** Items asked of the source at once. */
export const PAGE = 120;
/** Items read anew at most when the app comes back: the pages scrolled through, up to this. */
export const REREAD = 1200;
/** The id of a shot of the camera in the choice (it has no tile, and no access takes it away). */
export const CAPTURE_ID = 'capture:';

/**
 * Addresses of thumbnails the page keeps at most, the tiles' and the
 * viewer's together: fewer than the names of thumbnails the app's server
 * of files keeps (`MAX_THUMBS` = 512, media_server.rs), which forgets the
 * one handed out longest ago. An address kept here was handed out among
 * the last 384, so the server still knows it.
 */
export const THUMB_URLS = 384;

/** One item chosen: of the grid, or a shot of the camera (no tile of its own). */
export interface Chosen {
  id: string;
  path: string;
  kind: 'image' | 'video';
}

export class GalleryModel {
  items = $state.raw<MediaItem[]>([]);
  /** Every page is here. */
  done = $state(false);
  loading = $state(false);
  failed = $state(false);
  album = $state<string | undefined>(undefined);
  chosen = $state.raw<Chosen[]>([]);

  private cursor: string | undefined;
  // Moves with every new start (another album, the access changed): a page asked before is dropped.
  private run = 0;

  constructor(
    private readonly library: MediaLibrary,
    private readonly kind: MediaKind = 'media',
    /** At most this many chosen (an album holds 64). */
    readonly max = 64,
  ) {}

  /** From the newest again, of `album` (all when absent). */
  reset(album?: string) {
    this.run++;
    this.album = album;
    this.items = [];
    this.cursor = undefined;
    this.done = false;
    this.loading = false;
    this.failed = false;
  }

  /** The next page, unless one is on its way or all are here. */
  async more(): Promise<void> {
    if (this.loading || this.done) return;
    const run = this.run;
    this.loading = true;
    try {
      const page = await this.library.recent({ cursor: this.cursor, limit: PAGE, kind: this.kind, album: this.album });
      if (run !== this.run) return;
      const seen = new Set(this.items.map((i) => i.id));
      this.items = [...this.items, ...page.items.filter((i) => !seen.has(i.id))];
      this.cursor = page.cursor;
      this.done = !page.cursor || page.items.length === 0;
      this.failed = false;
    } catch {
      if (run === this.run) this.failed = true;
    } finally {
      if (run === this.run) this.loading = false;
    }
  }

  /**
   * The pages read anew in place (the app came back to the screen): as
   * many items as were here (at most `REREAD`), swapped in at once, so
   * the grid does not jump. What is no longer there leaves the grid; the
   * choice is not this one's to judge (`check`): an item out of the album
   * or the pages in view is not an item out of the access.
   */
  async refresh(): Promise<void> {
    const run = ++this.run;
    const want = Math.min(Math.max(this.items.length, PAGE), REREAD);
    this.loading = true;
    try {
      const items: MediaItem[] = [];
      const seen = new Set<string>();
      let cursor: string | undefined;
      let done = false;
      while (items.length < want) {
        const page = await this.library.recent({ cursor, limit: PAGE, kind: this.kind, album: this.album });
        if (run !== this.run) return;
        for (const i of page.items) {
          if (!seen.has(i.id)) {
            seen.add(i.id);
            items.push(i);
          }
        }
        cursor = page.cursor;
        if (!page.cursor || page.items.length === 0) {
          done = true;
          break;
        }
      }
      this.items = items;
      this.cursor = cursor;
      this.done = done;
      this.failed = false;
    } catch {
      if (run === this.run) this.failed = true;
    } finally {
      if (run === this.run) this.loading = false;
    }
  }

  /** Nothing of the gallery may be shown (no access): its items leave the grid and the choice; a shot stays. */
  shut() {
    this.reset(this.album);
    this.done = true;
    this.chosen = this.chosen.filter((c) => c.id.startsWith(CAPTURE_ID));
  }

  /**
   * Each item of the grid chosen asked of the source by itself (`readable`),
   * whatever album or page is shown: those it no longer lets the app read
   * leave the choice, the others stay where they were; a shot of the
   * camera is the app's own and stays. Chosen while it is asked: asked in
   * turn. When the source cannot tell, the choice stays as it is (it never
   * grows): what is out of the access fails at «Добавить» as an import does.
   */
  async check(): Promise<void> {
    const asked = new Set<string>();
    // A choice made while the source was asked is asked in turn; a few rounds at most.
    for (let round = 0; round < 4; round++) {
      const ask = this.chosen.filter((c) => !c.id.startsWith(CAPTURE_ID) && !asked.has(c.id));
      if (!ask.length) return;
      let ok: Set<string>;
      try {
        ok = new Set(await this.library.readable(ask.map((c) => c.path)));
      } catch {
        return;
      }
      for (const c of ask) asked.add(c.id);
      const gone = new Set(ask.filter((c) => !ok.has(c.path)).map((c) => c.id));
      if (gone.size) this.chosen = this.chosen.filter((c) => !gone.has(c.id));
    }
  }

  /** Whether `id` is among the items read. */
  has(id: string): boolean {
    return this.items.some((i) => i.id === id);
  }

  /** The place of an item in the choice, from 1; 0 when it is not chosen. */
  place(id: string): number {
    return this.chosen.findIndex((c) => c.id === id) + 1;
  }

  /** Chooses an item, or lets it go (the later ones move up); `false` when the choice is full. */
  toggle(item: Chosen): boolean {
    if (this.place(item.id)) {
      this.chosen = this.chosen.filter((c) => c.id !== item.id);
      return true;
    }
    if (this.chosen.length >= this.max) return false;
    this.chosen = [...this.chosen, { id: item.id, path: item.path, kind: item.kind }];
    return true;
  }

  clear() {
    this.chosen = [];
  }

  /** The paths chosen, in the order they were chosen. */
  paths(): string[] {
    return this.chosen.map((c) => c.path);
  }
}

/**
 * The large picture of one item of the grid. Every reading of the gallery
 * anew closes it first (`close`): its address goes, and an answer to an
 * opening asked before is dropped (`gen`), as `ThumbQueue` drops its own.
 */
export class Viewer {
  item = $state.raw<MediaItem | null>(null);
  url = $state('');
  /** Moves with every opening and closing: an older answer says nothing. */
  private gen = 0;

  constructor(
    private readonly library: MediaLibrary,
    private readonly thumbs: ThumbQueue,
  ) {}

  /** The small picture at once, the large one when it comes (within the same bound as the tiles). */
  async open(item: MediaItem): Promise<void> {
    const gen = ++this.gen;
    this.item = item;
    this.url = this.thumbs.urls[item.id] ?? '';
    try {
      const url = await this.thumbs.one(`view:${item.id}`, () => this.library.thumbUrl(item, 1280));
      if (gen === this.gen) this.url = url;
    } catch {
      // The small one stays.
    }
  }

  /** The large picture did not load: the small one, else nothing. */
  failed() {
    if (!this.item) return;
    const small = this.thumbs.urls[this.item.id] ?? '';
    this.url = this.url === small ? '' : small;
  }

  close() {
    this.gen++;
    this.item = null;
    this.url = '';
  }

  /** Whether the viewer is still what `open` made at `gen` (`now()` then). */
  now(): number {
    return this.gen;
  }
}

/** The rows of a grid of `rows` rows of `rowH` pixels that show in a view of `viewH` scrolled to `top`, with `over` rows around. */
export function rowWindow(top: number, viewH: number, rowH: number, rows: number, over = 3): { first: number; last: number } {
  if (rowH <= 0 || rows <= 0) return { first: 0, last: -1 };
  const first = Math.max(0, Math.floor(top / rowH) - over);
  const last = Math.min(rows - 1, Math.ceil((top + viewH) / rowH) + over);
  return { first, last };
}

/**
 * Thumbnails asked of the source a few at a time, the last wanted first:
 * a tile scrolled past before its turn is not asked at all, and one asked
 * again is answered from what came. At most `keep` addresses are kept
 * (the server of files forgets older ones): one that went is asked again
 * when its tile comes back. A picture the page could not load (its name
 * forgotten, its file gone) is asked again once (`failed`). `reset`
 * forgets every address: an answer asked before it is dropped.
 */
export class ThumbQueue {
  urls = $state.raw<Record<string, string>>({});
  private wanted = new Map<string, MediaItem>();
  private running = 0;
  /** What came, oldest first: past `keep` the oldest goes (the page does not hold every address of 10 000). */
  private order: string[] = [];
  /** The tiles on the screen, as last told. */
  private onScreen: MediaItem[] = [];
  /** How many times the picture of a tile failed since the last reset. */
  private failures = new Map<string, number>();
  /** Moves with every reset: an address asked before it is not kept. */
  private gen = 0;

  constructor(
    private readonly get: (item: MediaItem) => Promise<string>,
    private readonly at = 4,
    private readonly keep = THUMB_URLS,
  ) {}

  /** The tiles on the screen now: what is not among them is no longer asked. */
  want(items: MediaItem[]) {
    this.onScreen = items;
    this.wanted = new Map(items.filter((i) => !this.urls[i.id] && (this.failures.get(i.id) ?? 0) < 2).map((i) => [i.id, i]));
    this.pump();
  }

  /**
   * One address more, asked apart from the tiles (the large picture of
   * the viewer), kept under `key` within the same bound.
   */
  async one(key: string, ask: () => Promise<string>): Promise<string> {
    const gen = this.gen;
    const url = await ask();
    if (gen === this.gen) this.put(key, url);
    return url;
  }

  /** The picture at the address of `id` did not load: the address goes, and the tile is asked again once. */
  failed(id: string) {
    if (!this.urls[id]) return;
    this.failures.set(id, (this.failures.get(id) ?? 0) + 1);
    this.order = this.order.filter((x) => x !== id);
    const { [id]: _gone, ...rest } = this.urls;
    this.urls = rest;
    this.want(this.onScreen);
  }

  /** Every address forgotten (the gallery is read anew); the tiles on the screen are asked again. */
  reset() {
    this.gen++;
    this.order = [];
    this.failures.clear();
    this.urls = {};
    this.want(this.onScreen);
  }

  private pump() {
    while (this.running < this.at && this.wanted.size) {
      const last = [...this.wanted.keys()].pop()!;
      const item = this.wanted.get(last)!;
      this.wanted.delete(last);
      this.running++;
      const gen = this.gen;
      this.get(item)
        .then((url) => { if (gen === this.gen) this.put(item.id, url); }, () => {})
        .finally(() => {
          this.running--;
          this.pump();
        });
    }
  }

  private put(id: string, url: string) {
    this.order = [...this.order.filter((x) => x !== id), id];
    const next = { ...this.urls, [id]: url };
    while (this.order.length > this.keep) delete next[this.order.shift()!];
    this.urls = next;
  }
}

/**
 * The gallery read anew, as the panel does when it opens and whenever the
 * app comes back to the screen (from the settings, the camera, the
 * system's chooser): the access may have changed, and whatever it says —
 * the same word too, as the set a partial access lets in may have changed
 * (A → B, photos → videos) — the same steps follow:
 *
 * 1. the viewer is closed at once, before anything is asked, and its
 *    address goes; an answer to an opening asked before is dropped;
 * 2. the addresses of thumbnails handed out are forgotten by the app's
 *    server of files and here;
 * 3. the pages are read again (in place when the access stays: the grid
 *    does not jump), or the grid empties (no access);
 * 4. each chosen item is asked of the source by itself (`check`), whatever
 *    album or page is shown; a viewer opened meanwhile, too.
 *
 * A step that fails leaves the viewer closed and the choice no larger.
 * The access now.
 */
export async function reread(
  library: MediaLibrary,
  model: GalleryModel,
  thumbs: ThumbQueue,
  viewer: Viewer,
  before: MediaAccess | null,
): Promise<MediaAccess> {
  viewer.close();
  const opened = viewer.now();
  await library.forgetThumbs().catch(() => {});
  thumbs.reset();
  let now: MediaAccess;
  try {
    now = await library.access();
  } catch {
    now = 'denied';
  }
  if (now === 'denied') {
    model.shut();
  } else if (now !== before || !model.items.length) {
    model.reset(model.album);
    await model.more();
  } else {
    await model.refresh();
  }
  if (now !== 'denied') await model.check();
  // Opened while the gallery was read, of what it showed before: kept only when it may still be read.
  const item = viewer.item;
  if (item && viewer.now() !== opened) {
    const gen = viewer.now();
    let ok = false;
    if (now !== 'denied') {
      try {
        ok = (await library.readable([item.path])).includes(item.path);
      } catch {
        ok = false;
      }
    }
    if (!ok && viewer.now() === gen) viewer.close();
  }
  return now;
}

/** `0:05`, `12:34`, `1:02:03`: the length of a video on its tile. */
export function durationLabel(seconds: number | undefined): string {
  if (seconds === undefined || !Number.isFinite(seconds) || seconds < 0) return '';
  const s = Math.round(seconds);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

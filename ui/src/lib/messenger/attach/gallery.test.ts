// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The grid of the tab «Галерея» (ROO-110 1b): its pages, the choice and its
// numbers, which rows are drawn, how thumbnails are asked, and what each
// source of media says it can do.

import { describe, expect, it, vi } from 'vitest';
import { CAPTURE_ID, GalleryModel, PAGE, REREAD, THUMB_URLS, ThumbQueue, Viewer, durationLabel, reread, rowWindow } from './gallery.svelte';
import { makeMockLibrary, mockItem, MOCK_COUNT } from './MockLibrary';
import { noGrid, type MediaItem, type MediaLibrary } from './MediaLibrary';
import { systemPickerLibrary } from './SystemPickerLibrary';
import { androidMediaLibrary } from './AndroidMediaLibrary';
import panelSource from './GalleryPanel.svelte?raw';
import sheetSource from './AttachSheet.svelte?raw';
import builtinSource from './builtin.ts?raw';
import librarySource from './library.ts?raw';

/** A source whose pages wait until the test lets them go. */
function slowLibrary(total: number) {
  const asked: { cursor?: string; album?: string; release: () => void }[] = [];
  const lib: MediaLibrary = {
    ...makeMockLibrary('full', total),
    recent(o) {
      return new Promise((resolve) => {
        const start = o.cursor ? Number(o.cursor) : 0;
        const items = Array.from({ length: Math.max(0, Math.min(o.limit, total - start)) }, (_, k) => mockItem(start + k));
        const end = start + items.length;
        asked.push({ cursor: o.cursor, album: o.album, release: () => resolve({ items, cursor: end < total ? String(end) : undefined }) });
      });
    },
  };
  return { lib, asked };
}

describe('the pages of the grid', () => {
  it('come one after the other by the cursor, to the last, and no further', async () => {
    const g = new GalleryModel(makeMockLibrary('full', 300));
    await g.more();
    expect(g.items.length).toBe(PAGE);
    expect(g.done).toBe(false);
    await g.more();
    await g.more();
    expect(g.items.length).toBe(300);
    expect(g.done).toBe(true);
    expect(new Set(g.items.map((i) => i.id)).size).toBe(300);
    await g.more();
    expect(g.items.length).toBe(300);
  });

  it('are asked once at a time: a second call while one is on its way asks nothing', async () => {
    const { lib, asked } = slowLibrary(1000);
    const g = new GalleryModel(lib);
    const a = g.more();
    const b = g.more();
    expect(asked.length).toBe(1);
    asked[0].release();
    await Promise.all([a, b]);
    expect(g.items.length).toBe(PAGE);
  });

  it('of another album start over, and a page of the album left is dropped', async () => {
    const { lib, asked } = slowLibrary(1000);
    const g = new GalleryModel(lib);
    const late = g.more();
    g.reset('camera');
    const now = g.more();
    expect(asked.map((a) => a.album)).toEqual([undefined, 'camera']);
    asked[1].release();
    await now;
    asked[0].release();
    await late;
    expect(g.album).toBe('camera');
    expect(g.items.length).toBe(PAGE);
    expect(g.loading).toBe(false);
  });

  it('that fail say so, and the next ask tries again', async () => {
    let fail = true;
    const base = makeMockLibrary('full', 10);
    const g = new GalleryModel({ ...base, recent: (o) => (fail ? Promise.reject(new Error('x')) : base.recent(o)) });
    await g.more();
    expect(g.failed).toBe(true);
    fail = false;
    await g.more();
    expect(g.failed).toBe(false);
    expect(g.items.length).toBe(10);
  });

  it('of ten thousand photos are read a page at a time, never all at once', async () => {
    const lib = makeMockLibrary('full');
    const recent = vi.spyOn(lib, 'recent');
    const g = new GalleryModel(lib);
    await g.more();
    expect(recent).toHaveBeenCalledTimes(1);
    expect(recent.mock.calls[0][0].limit).toBe(PAGE);
    expect(g.items.length).toBe(PAGE);
    expect(MOCK_COUNT).toBeGreaterThanOrEqual(10_000);
  });

  it('show only what a partial access lets through, and nothing when it is denied', async () => {
    const part = new GalleryModel(makeMockLibrary('partial', 1000));
    await part.more();
    await part.more();
    expect(part.items.length).toBe(20);
    const none = makeMockLibrary('denied', 1000);
    expect(await none.access()).toBe('denied');
    expect(await none.request()).toBe('full');
  });
});

describe('the choice', () => {
  const pick = (i: number) => {
    const m = mockItem(i);
    return { id: m.id, path: m.path, kind: m.kind };
  };

  it('numbers the items in the order they were chosen; one let go makes the later ones move up', () => {
    const g = new GalleryModel(makeMockLibrary('full', 10));
    g.toggle(pick(5));
    g.toggle(pick(2));
    g.toggle(pick(8));
    expect([g.place('m5'), g.place('m2'), g.place('m8'), g.place('m1')]).toEqual([1, 2, 3, 0]);
    g.toggle(pick(5));
    expect([g.place('m5'), g.place('m2'), g.place('m8')]).toEqual([0, 1, 2]);
    expect(g.paths()).toEqual([pick(2).path, pick(8).path]);
  });

  it('holds no more than the composer takes', () => {
    const g = new GalleryModel(makeMockLibrary('full', 10), 'media', 2);
    expect(g.toggle(pick(1))).toBe(true);
    expect(g.toggle(pick(2))).toBe(true);
    expect(g.toggle(pick(3))).toBe(false);
    expect(g.chosen.length).toBe(2);
    // Letting one go is always possible.
    expect(g.toggle(pick(1))).toBe(true);
    expect(g.toggle(pick(3))).toBe(true);
  });

  it('keeps what was chosen across albums and pages', async () => {
    const g = new GalleryModel(makeMockLibrary('full', 500));
    await g.more();
    g.toggle(pick(3));
    g.reset('screenshots');
    await g.more();
    expect(g.place('m3')).toBe(1);
    g.clear();
    expect(g.chosen).toEqual([]);
  });
});

describe('the rows on the screen', () => {
  it('are those in view with a few around, within the grid', () => {
    expect(rowWindow(0, 400, 120, 1000)).toEqual({ first: 0, last: 7 });
    expect(rowWindow(12_000, 400, 120, 1000)).toEqual({ first: 97, last: 107 });
    expect(rowWindow(119_600, 400, 120, 1000)).toEqual({ first: 993, last: 999 });
    expect(rowWindow(0, 400, 120, 0)).toEqual({ first: 0, last: -1 });
  });

  it('stay as many wherever the grid is scrolled to: ten thousand photos draw no more tiles than one screen', () => {
    const rows = Math.ceil(10_001 / 3);
    const sizes = [0, 5_000, 200_000, rows * 120 - 400].map((top) => {
      const w = rowWindow(top, 400, 120, rows);
      return w.last - w.first + 1;
    });
    expect(Math.max(...sizes)).toBeLessThanOrEqual(11);
  });
});

describe('the thumbnails', () => {
  const items = (from: number, n: number): MediaItem[] => Array.from({ length: n }, (_, k) => mockItem(from + k));

  it('are asked a few at a time, the last wanted first, and those scrolled past are not asked', async () => {
    const waiting: { id: string; done: (u: string) => void }[] = [];
    const q = new ThumbQueue((it) => new Promise((done) => waiting.push({ id: it.id, done })), 2);
    q.want(items(0, 6));
    expect(waiting.map((w) => w.id)).toEqual(['m5', 'm4']);
    // Scrolled on: m0…m3 are no longer on the screen.
    q.want(items(100, 3));
    waiting.splice(0).forEach((w) => w.done(`url-${w.id}`));
    await new Promise((r) => setTimeout(r));
    expect(waiting.map((w) => w.id).sort()).toEqual(['m101', 'm102']);
    waiting.splice(0).forEach((w) => w.done(`url-${w.id}`));
    await new Promise((r) => setTimeout(r));
    expect(waiting.map((w) => w.id)).toEqual(['m100']);
    waiting.splice(0).forEach((w) => w.done(`url-${w.id}`));
    await new Promise((r) => setTimeout(r));
    expect(Object.keys(q.urls).sort()).toEqual(['m100', 'm101', 'm102', 'm4', 'm5']);
    // What came is not asked again.
    q.want(items(100, 3));
    expect(waiting).toEqual([]);
  });

  it('keep a bounded number of addresses: the oldest go', async () => {
    const q = new ThumbQueue(async (it) => `url-${it.id}`, 4, 10);
    for (let k = 0; k < 50; k += 5) {
      q.want(items(k, 5));
      await new Promise((r) => setTimeout(r, 1));
    }
    expect(Object.keys(q.urls).length).toBe(10);
    expect(q.urls.m49).toBe('url-m49');
    expect(q.urls.m0).toBeUndefined();
  });

  /**
   * The server of files as media_server.rs keeps the names of thumbnails:
   * 512, the one handed out longest ago forgotten first, one handed out
   * again the newest; `forgetAll` as `forget_thumbs`.
   */
  function thumbServer(max = 512) {
    let names: string[] = [];
    let n = 0;
    const byItem = new Map<string, string>();
    return {
      asked: 0,
      async thumbUrl(item: MediaItem, px = 0) {
        this.asked++;
        const key = `${item.id}@${px > 600 ? 'big' : 'tile'}`;
        let name = byItem.get(key);
        if (!name || !names.includes(name)) {
          name = `t${++n}`;
          byItem.set(key, name);
        }
        names = [...names.filter((x) => x !== name), name];
        while (names.length > max) names.shift();
        return `http://127.0.0.1/${name}`;
      },
      live: (url: string) => names.includes(url.split('/').pop()!),
      forgetAll() { names = []; },
    };
  }

  it('keep fewer addresses than the server of files names, so none they keep is forgotten there, scrolled forward and back', async () => {
    expect(THUMB_URLS).toBeLessThan(512);
    const server = thumbServer();
    const q = new ThumbQueue((it) => server.thumbUrl(it), 4);
    const screens: number[] = [];
    for (let k = 0; k < 1500; k += 24) screens.push(k);
    for (const k of [...screens, ...screens.reverse()]) {
      q.want(items(k, 24));
      await new Promise((r) => setTimeout(r, 1));
      // The viewer opened now and then: its large picture counts in the same bound.
      if (k % 240 === 0) await q.one(`view:m${k}`, () => server.thumbUrl(mockItem(k), 1280));
      const kept = Object.values(q.urls);
      expect(kept.length).toBeLessThanOrEqual(THUMB_URLS);
      expect(kept.filter((u) => !server.live(u))).toEqual([]);
    }
    // Back at the top, the first screen was asked again (its addresses had gone), and shows.
    for (const it of items(0, 24)) expect(server.live(q.urls[it.id])).toBe(true);
  });

  it('whose picture does not load are asked again once, then left empty', async () => {
    let n = 0;
    const get = vi.fn(async (it: MediaItem) => `url-${it.id}-${++n}`);
    const q = new ThumbQueue(get, 4);
    q.want(items(0, 2));
    await new Promise((r) => setTimeout(r, 1));
    expect(q.urls.m0).toBe('url-m0-2');
    q.failed('m0');
    await new Promise((r) => setTimeout(r, 1));
    expect(q.urls.m0).toBe('url-m0-3');
    q.failed('m0');
    await new Promise((r) => setTimeout(r, 1));
    expect(q.urls.m0).toBeUndefined();
    expect(get).toHaveBeenCalledTimes(3);
    // A reset asks it again.
    q.reset();
    await new Promise((r) => setTimeout(r, 1));
    expect(Object.keys(q.urls).sort()).toEqual(['m0', 'm1']);
  });

  it('asked before a reset are dropped when they come', async () => {
    const waiting: ((u: string) => void)[] = [];
    const q = new ThumbQueue(() => new Promise((done) => waiting.push(done)), 4);
    q.want(items(0, 1));
    q.reset();
    waiting[0]('stale');
    await new Promise((r) => setTimeout(r, 1));
    expect(q.urls.m0).toBeUndefined();
    waiting[1]('fresh');
    await new Promise((r) => setTimeout(r, 1));
    expect(q.urls.m0).toBe('fresh');
  });

  it('of the phone are addresses of the app, never base64', () => {
    expect(androidMediaLibrary.thumbUrl.toString()).toContain('gallery.thumb');
    expect(panelSource).not.toMatch(/base64/);
  });
});

describe('the sources of media', () => {
  it('say what they can do: the phone a grid, the system picker none but the camera and the Photo Picker', () => {
    expect(androidMediaLibrary.capabilities).toEqual({ pickSystem: true, grid: true, capture: true, photoPicker: true });
    expect(systemPickerLibrary.capabilities).toEqual({ pickSystem: true, grid: false, capture: true, photoPicker: true });
    expect(makeMockLibrary('picker').capabilities.grid).toBe(false);
  });

  it('are chosen in one place, and the panel asks the source what it can do, never its name', () => {
    expect(librarySource).toContain('PHONE_GRID ? androidMediaLibrary : systemPickerLibrary');
    expect(panelSource).toContain('library.capabilities');
    expect(panelSource).not.toMatch(/androidMediaLibrary|systemPickerLibrary|dialogLibrary|mockLibrary/);
  });

  it('without a grid say nothing of one', async () => {
    expect(await systemPickerLibrary.access()).toBe('denied');
    expect(await systemPickerLibrary.recent({ limit: 10 })).toEqual({ items: [] });
    expect(await noGrid.capture()).toBeNull();
  });

  it('give «Файл» as files, as they are, and the gallery with its «без сжатия»', () => {
    expect(builtinSource).toContain("asFile: true");
    expect(panelSource).toMatch(/done\(\{ type: 'files', paths: model\.paths\(\), original \}\)/);
  });
});

describe('the gallery read anew when the app comes back', () => {
  /**
   * A phone's gallery whose access the test changes as the settings would:
   * `lets` what the access lets in, `album` the album of an item (`a` and
   * `b`, by halves of ten); `readable` asks each item by itself, as the
   * plugin asks MediaStore for its row.
   */
  function phone(total = 400) {
    const state = {
      access: 'partial' as 'full' | 'partial' | 'denied',
      lets: (i: number) => i < 3,
      failRecent: false,
      failReadable: false,
    };
    const forgot = vi.fn(async () => {});
    const albumOf = (i: number) => (Math.floor(i / 10) % 2 ? 'b' : 'a');
    const indexOf = (path: string) => Number(/-(\d+)\./.exec(path)![1]);
    const asked: string[][] = [];
    const lib: MediaLibrary = {
      ...makeMockLibrary('full', total),
      access: async () => state.access,
      forgetThumbs: forgot,
      async recent({ cursor, limit, album }) {
        if (state.failRecent) throw new Error('MediaStore is gone');
        const out: MediaItem[] = [];
        let i = cursor ? Number(cursor) : 0;
        for (; i < total && out.length < limit; i++) {
          if (state.access !== 'denied' && state.lets(i) && (!album || albumOf(i) === album)) out.push(mockItem(i));
        }
        return { items: out, cursor: i < total ? String(i) : undefined };
      },
      async readable(paths) {
        asked.push(paths);
        if (state.failReadable) throw new Error('MediaStore is gone');
        return paths.filter((p) => state.access !== 'denied' && state.lets(indexOf(p)));
      },
    };
    return { lib, state, forgot, asked };
  }

  const pick = (g: GalleryModel, i: number) => g.toggle({ id: `m${i}`, path: mockItem(i).path, kind: mockItem(i).kind });
  const tick = () => new Promise((r) => setTimeout(r, 1));

  it('shows the new set of a partial access that stays partial (A → B), and forgets the addresses of the old one', async () => {
    const { lib, state, forgot } = phone();
    const g = new GalleryModel(lib);
    const server = { n: 0 };
    const q = new ThumbQueue(async (it) => `url-${it.id}-${++server.n}`, 4);
    const v = new Viewer(lib, q);
    let access = await reread(lib, g, q, v, null);
    expect(access).toBe('partial');
    expect(g.items.map((i) => i.id)).toEqual(['m0', 'm1', 'm2']);
    q.want(g.items);
    await new Promise((r) => setTimeout(r, 1));
    const before = { ...q.urls };
    g.toggle({ id: 'm1', path: mockItem(1).path, kind: 'image' });
    g.toggle({ id: `${CAPTURE_ID}content://shot`, path: 'content://shot', kind: 'image' });

    // In the settings the user lets in another set: m5 and m7, still «partial».
    state.lets = (i) => i === 5 || i === 7;
    q.want([]);
    access = await reread(lib, g, q, v, access);
    expect(access).toBe('partial');
    expect(g.items.map((i) => i.id)).toEqual(['m5', 'm7']);
    expect(g.chosen.map((c) => c.id)).toEqual([`${CAPTURE_ID}content://shot`]);
    expect(g.has('m1')).toBe(false);
    expect(Object.keys(q.urls)).toEqual([]);
    expect(forgot).toHaveBeenCalledTimes(2);
    expect(Object.values(before).length).toBe(3);
  });

  it('shows the videos of a partial access that went from photos to videos (Android 13)', async () => {
    const { lib, state } = phone(30);
    state.lets = (i) => mockItem(i).kind === 'image';
    const g = new GalleryModel(lib);
    const q = new ThumbQueue(async (it) => it.id, 4);
    const v = new Viewer(lib, q);
    let access = await reread(lib, g, q, v, null);
    expect(g.items.every((i) => i.kind === 'image')).toBe(true);
    g.toggle({ id: 'm0', path: mockItem(0).path, kind: 'image' });
    state.lets = (i) => mockItem(i).kind === 'video';
    access = await reread(lib, g, q, v, access);
    expect(access).toBe('partial');
    expect(g.items.length).toBeGreaterThan(0);
    expect(g.items.every((i) => i.kind === 'video')).toBe(true);
    expect(g.chosen).toEqual([]);
  });

  it('keeps the pages scrolled through in place when nothing changed, and empties the grid when the access is gone', async () => {
    const { lib, state } = phone(1000);
    state.access = 'full';
    state.lets = () => true;
    const g = new GalleryModel(lib);
    const q = new ThumbQueue(async (it) => it.id, 4);
    const v = new Viewer(lib, q);
    let access = await reread(lib, g, q, v, null);
    await g.more();
    await g.more();
    expect(g.items.length).toBe(3 * PAGE);
    g.toggle({ id: 'm300', path: mockItem(300).path, kind: 'image' });
    access = await reread(lib, g, q, v, access);
    expect(g.items.length).toBe(3 * PAGE);
    expect(g.place('m300')).toBe(1);

    state.access = 'denied';
    access = await reread(lib, g, q, v, access);
    expect(access).toBe('denied');
    expect(g.items).toEqual([]);
    expect(g.chosen).toEqual([]);
  });

  it('keeps a choice made in two albums when the app comes back, from the camera too', async () => {
    const { lib, state, asked } = phone(100);
    state.access = 'full';
    state.lets = () => true;
    const g = new GalleryModel(lib);
    const q = new ThumbQueue(async (it) => it.id, 4);
    const v = new Viewer(lib, q);
    let access = await reread(lib, g, q, v, null);
    g.reset('a');
    await g.more();
    pick(g, 2);
    g.reset('b');
    await g.more();
    expect(g.has('m2')).toBe(false);
    pick(g, 13);
    // The camera: the shot joins the choice, then the app comes back to the screen.
    g.toggle({ id: `${CAPTURE_ID}content://shot`, path: 'content://shot', kind: 'image' });
    access = await reread(lib, g, q, v, access);
    expect(g.album).toBe('b');
    expect(g.chosen.map((c) => c.id)).toEqual(['m2', 'm13', `${CAPTURE_ID}content://shot`]);
    // Each item of the grid asked by itself; the shot is the app's own.
    expect(asked.at(-1)).toEqual([mockItem(2).path, mockItem(13).path]);
    // Read again with nothing changed: nothing leaves.
    access = await reread(lib, g, q, v, access);
    expect(g.chosen.length).toBe(3);
  });

  it('lets go of what a partial access no longer holds when the access changes word (full → partial B)', async () => {
    const { lib, state } = phone(100);
    state.access = 'full';
    state.lets = () => true;
    const g = new GalleryModel(lib);
    const q = new ThumbQueue(async (it) => it.id, 4);
    const v = new Viewer(lib, q);
    let access = await reread(lib, g, q, v, null);
    pick(g, 0);
    state.access = 'partial';
    state.lets = (i) => i >= 50;
    access = await reread(lib, g, q, v, access);
    expect(access).toBe('partial');
    expect(g.items[0].id).toBe('m50');
    expect(g.chosen).toEqual([]);
  });

  it('closes the viewer at once and lets go of A when a partial access holds B of more than a page (partial A → partial B)', async () => {
    const { lib, state } = phone(2000);
    state.lets = (i) => i < 3;
    const g = new GalleryModel(lib);
    let n = 0;
    const q = new ThumbQueue(async (it) => `url-${it.id}-${++n}`, 4);
    const v = new Viewer(lib, q);
    let access = await reread(lib, g, q, v, null);
    pick(g, 0);
    await v.open(mockItem(0));
    expect(v.item?.id).toBe('m0');
    expect(v.url).toBe(q.urls['view:m0']);
    expect(v.url).not.toBe('');

    // B: everything from 1000 on, more than a page and more than is read again.
    state.lets = (i) => i >= 1000;
    const p = reread(lib, g, q, v, access);
    // Closed before anything was asked, its address gone.
    expect(v.item).toBeNull();
    expect(v.url).toBe('');
    access = await p;
    expect(access).toBe('partial');
    expect(g.done).toBe(false);
    expect(g.items.length).toBeGreaterThanOrEqual(PAGE);
    expect(g.items.length).toBeLessThanOrEqual(REREAD);
    expect(g.items.every((i) => Number(i.id.slice(1)) >= 1000)).toBe(true);
    expect(g.chosen).toEqual([]);
    expect(Object.keys(q.urls).filter((k) => k.includes('m0'))).toEqual([]);
  });

  it('drops the answer to an opening of the viewer asked before the gallery was read anew', async () => {
    const { lib } = phone();
    const g = new GalleryModel(lib);
    const waiting: ((u: string) => void)[] = [];
    const q = new ThumbQueue(async (it) => it.id, 4);
    const slow: MediaLibrary = { ...lib, thumbUrl: () => new Promise((done) => waiting.push(done)) };
    const v = new Viewer(slow, q);
    let access = await reread(lib, g, q, v, null);
    const opening = v.open(mockItem(1));
    access = await reread(lib, g, q, v, access);
    expect(v.item).toBeNull();
    waiting[0]('http://127.0.0.1/stale');
    await opening;
    expect(v.item).toBeNull();
    expect(v.url).toBe('');
    expect(Object.values(q.urls)).not.toContain('http://127.0.0.1/stale');
    expect(access).toBe('partial');
  });

  it('closes a viewer opened while the gallery was read, of an item it no longer holds', async () => {
    const { lib, state } = phone();
    const g = new GalleryModel(lib);
    const q = new ThumbQueue(async (it) => it.id, 4);
    const v = new Viewer(lib, q);
    let access = await reread(lib, g, q, v, null);
    state.lets = (i) => i === 5;
    const p = reread(lib, g, q, v, access);
    // A tile of the old grid tapped meanwhile.
    void v.open(mockItem(1));
    access = await p;
    expect(v.item).toBeNull();
    // One of the new set stays open.
    const p2 = reread(lib, g, q, v, access);
    void v.open(mockItem(5));
    await p2;
    expect(v.item?.id).toBe('m5');
  });

  it('asks in turn what was chosen while the source was asked', async () => {
    const { lib, state } = phone(100);
    state.access = 'full';
    state.lets = () => true;
    const g = new GalleryModel(lib);
    const q = new ThumbQueue(async (it) => it.id, 4);
    const v = new Viewer(lib, q);
    let access = await reread(lib, g, q, v, null);
    pick(g, 1);
    state.lets = (i) => i !== 1 && i !== 2;
    const p = reread(lib, g, q, v, access);
    // A tile of the old grid chosen before the new one came.
    pick(g, 2);
    access = await p;
    expect(g.chosen).toEqual([]);
  });

  it('when the gallery cannot be read again: the viewer closed, the choice no larger, and «Добавить» gives what is left', async () => {
    const { lib, state } = phone(100);
    state.access = 'full';
    state.lets = () => true;
    const g = new GalleryModel(lib);
    const q = new ThumbQueue(async (it) => it.id, 4);
    const v = new Viewer(lib, q);
    let access = await reread(lib, g, q, v, null);
    pick(g, 0);
    pick(g, 7);
    await v.open(mockItem(7));

    // The pages fail: the viewer is closed, the source still tells each chosen item.
    state.failRecent = true;
    state.lets = (i) => i !== 7;
    access = await reread(lib, g, q, v, access);
    expect(g.failed).toBe(true);
    expect(v.item).toBeNull();
    expect(g.chosen.map((c) => c.id)).toEqual(['m0']);

    // The source cannot tell either: what is chosen stays as it is, and nothing is added.
    pick(g, 9);
    state.failReadable = true;
    state.lets = (i) => i === 0;
    await v.open(mockItem(0));
    access = await reread(lib, g, q, v, access);
    expect(v.item).toBeNull();
    expect(g.chosen.map((c) => c.id)).toEqual(['m0', 'm9']);
    // «Добавить» hands the paths as they are; m9 fails at the import as any unreadable file does.
    expect(g.paths()).toEqual([mockItem(0).path, mockItem(9).path]);

    // The access itself cannot be read: no access, nothing of the gallery stays.
    lib.access = async () => { throw new Error('gone'); };
    g.toggle({ id: `${CAPTURE_ID}content://shot`, path: 'content://shot', kind: 'image' });
    access = await reread(lib, g, q, v, access);
    expect(access).toBe('denied');
    expect(g.chosen.map((c) => c.id)).toEqual([`${CAPTURE_ID}content://shot`]);
  });

  it('is what the panel does on every return, the same access too', () => {
    const script = panelSource.split('</script>')[0];
    const read = script.split('async function readAccess()')[1].split('\n  }\n')[0];
    expect(read).toContain('reread(library, model, thumbs, viewer, access)');
    expect(read).not.toMatch(/now === access\)\s*return/);
    expect(panelSource).toContain('onerror={() => thumbs.failed(item.id)}');
    // The viewer is the model's (`Viewer`), not a copy of the panel's.
    expect(script).toContain('new Viewer(library, thumbs)');
    expect(script).not.toMatch(/let view(ing|Url)\b/);
    expect(panelSource).toContain('onclick={() => viewer.open(item)}');
  });

  it('of the phone asks each chosen item by itself, of the plugin', () => {
    expect(androidMediaLibrary.readable.toString()).toContain('gallery.readable');
  });
});

describe('the sheet', () => {
  it('asks nothing of the user before a tap: the panel reads the access, the request is a button', () => {
    const script = panelSource.split('</script>')[0];
    const mount = script.split('onMount(')[1].split('});')[0];
    expect(mount).toContain('readAccess');
    expect(mount).not.toContain('request');
    expect(panelSource).toMatch(/onclick=\{allow\}/);
  });

  it('puts the bar of the panel (a choice) where the strip stands, and takes it back with the panel', () => {
    expect(sheetSource).toContain('{#if panelBar && active}');
    expect(sheetSource).toMatch(/ticket = r \? gate\.panel\(r\) : null;\s+panelBar = null;/);
    expect(sheetSource).toContain('if (ticket === t) panelBar = b;');
  });
});

describe('the length of a video', () => {
  it('reads as on a phone', () => {
    expect(durationLabel(5)).toBe('0:05');
    expect(durationLabel(754.4)).toBe('12:34');
    expect(durationLabel(3723)).toBe('1:02:03');
    expect(durationLabel(undefined)).toBe('');
    expect(durationLabel(Number.NaN)).toBe('');
  });
});

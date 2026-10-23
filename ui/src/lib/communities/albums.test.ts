// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Files sent together are one post of the feed; files picked wait in the
// tray and go as one album with the caption on the last.

import { describe, expect, it, vi } from 'vitest';
import type { FeedItemView } from './api';
import { MAX_ALBUM, albumParts, feedUnits } from './albums';
import { Picked, newBatch, pathName } from './picked.svelte';

const T = 1_800_000_000;
let n = 0;
function post(extra: Partial<FeedItemView> & { kind?: string; batch?: string } = {}): FeedItemView {
  const { kind, batch, ...rest } = extra;
  n += 1;
  return {
    id: `p${String(n).padStart(3, '0')}`, community_id: 'c', post_id: '', author: 'a', created_at: T + n, review: 'ok', hidden: false, deleted: false,
    pinned: false, reactions: [], comments: 0, pending: false,
    media_json: kind ? JSON.stringify({ v: 1, t: 'media', kind, name: `f${n}`, mime: '', size: 1, batch }) : undefined,
    ...rest,
  };
}
const sizes = (xs: FeedItemView[]) => feedUnits(xs).map((u) => u.items.length);

describe('the albums of a feed', () => {
  it('never gather a voice message or a circle, whatever batch they name', () => {
    expect(sizes([post({ kind: 'voice', batch: 'b9-aaaaaaaa' }), post({ kind: 'voice', batch: 'b9-aaaaaaaa' })])).toEqual([1, 1]);
    expect(sizes([post({ kind: 'image', batch: 'b8-aaaaaaaa' }), post({ kind: 'circle', batch: 'b8-aaaaaaaa' })])).toEqual([1, 1]);
  });

  it('gather consecutive posts of one author with one batch', () => {
    const xs = [post({ text: 'hi' }), post({ kind: 'image', batch: 'b1-aaaaaaaa' }), post({ kind: 'image', batch: 'b1-aaaaaaaa' }), post({ kind: 'video', batch: 'b1-aaaaaaaa', text: 'caption' }), post({ text: 'after' })];
    const units = feedUnits(xs);
    expect(units.map((u) => u.items.length)).toEqual([1, 3, 1]);
    // The album keeps the id of its first post as its key, whatever comes after.
    expect(units[1].key).toBe(xs[1].id);
    expect(units[1].items.at(-1)?.text).toBe('caption');
  });

  it('do not gather other authors, other batches, files alone or posts in between', () => {
    expect(sizes([post({ kind: 'image', batch: 'b2-aaaaaaaa' }), post({ kind: 'image', batch: 'b2-aaaaaaaa', author: 'b' })])).toEqual([1, 1]);
    expect(sizes([post({ kind: 'image', batch: 'b3-aaaaaaaa' }), post({ kind: 'image', batch: 'b4-aaaaaaaa' })])).toEqual([1, 1]);
    expect(sizes([post({ kind: 'image' }), post({ kind: 'image' })])).toEqual([1, 1]);
    expect(sizes([post({ kind: 'image', batch: 'b5-aaaaaaaa' }), post({ text: 'x' }), post({ kind: 'image', batch: 'b5-aaaaaaaa' })])).toEqual([1, 1, 1]);
    expect(sizes([post({ kind: 'image', batch: 'b6-aaaaaaaa' }), post({ kind: 'image', batch: 'b6-aaaaaaaa', deleted: true })])).toEqual([1, 1]);
  });

  it('do not gather across a day', () => {
    const a = post({ kind: 'image', batch: 'b7-aaaaaaaa' });
    const b = post({ kind: 'image', batch: 'b7-aaaaaaaa', created_at: a.created_at + 2 * 86_400 });
    expect(sizes([a, b])).toEqual([1, 1]);
  });

  it('hold ten at most', () => {
    const xs = Array.from({ length: MAX_ALBUM + 2 }, () => post({ kind: 'image', batch: 'b8-aaaaaaaa' }));
    expect(sizes(xs)).toEqual([MAX_ALBUM, 2]);
  });

  it('show pictures as a mosaic and other files as a list', () => {
    const xs = [post({ kind: 'image', batch: 'b9-aaaaaaaa' }), post({ kind: 'file', batch: 'b9-aaaaaaaa' }), post({ kind: 'video', batch: 'b9-aaaaaaaa' })];
    const { visual, files } = albumParts(xs);
    expect(visual.map((x) => x.id)).toEqual([xs[0].id, xs[2].id]);
    expect(files.map((x) => x.id)).toEqual([xs[1].id]);
  });
});

describe('the files picked', () => {
  it('are named by their path, a phone\'s too', () => {
    expect(pathName('/home/me/Pictures/a b.jpg')).toBe('a b.jpg');
    expect(pathName('content://media/external/IMG%201.jpg')).toBe('IMG 1.jpg');
    expect(pathName('C:\\Users\\me\\x.png')).toBe('x.png');
  });

  it('make a batch the messenger takes', () => {
    expect(newBatch()).toMatch(/^[0-9A-Za-z_-]{8,64}$/);
  });

  it('go as one album, the caption with the last', async () => {
    const picked = new Picked();
    picked.add(['/a.jpg', '/b.jpg', '/c.jpg']);
    const go = vi.fn(async () => {});
    await picked.send(go, 'three');
    expect(go).toHaveBeenCalledTimes(3);
    const batches = go.mock.calls.map((c) => (c as unknown as [string, { batch?: string }])[1].batch);
    expect(new Set(batches).size).toBe(1);
    expect(batches[0]).toBeTruthy();
    expect(go.mock.calls.map((c) => (c as unknown as [string, { caption?: string }])[1].caption)).toEqual([undefined, undefined, 'three']);
    expect(picked.files).toEqual([]);
  });

  it('go alone without a batch', async () => {
    const picked = new Picked();
    picked.add(['/a.jpg']);
    const go = vi.fn(async () => {});
    await picked.send(go, 'one');
    expect(go).toHaveBeenCalledWith('/a.jpg', { caption: 'one', batch: undefined });
  });

  it('give the caption its format, with the last file alone', async () => {
    const picked = new Picked();
    picked.add(['/a.jpg', '/b.pdf']);
    const go = vi.fn(async () => {});
    await picked.send(go, '# Title\n**words**', 'md');
    const opts = go.mock.calls.map((c) => (c as unknown as [string, { caption?: string; fmt?: string }])[1]);
    expect(opts.map((o) => [o.caption, o.fmt])).toEqual([[undefined, undefined], ['# Title\n**words**', 'md']]);
  });

  it('send a video as a file of the device with a frame of it, and as it is when none can be taken', async () => {
    const poster = { jpeg: 'AAAA', width: 1920, height: 1080 };
    const videoOf = vi.fn(async (path: string) => (path === '/bad.mp4' ? Promise.reject(new Error('no')) : { path: '/outgoing/1-clip.mp4', poster }));
    const picked = new Picked(async () => null, videoOf);
    picked.add(['content://media/clip.mp4', '/bad.mp4', '/a.jpg']);
    const go = vi.fn(async () => {});
    await picked.send(go);
    expect(go.mock.calls.map((c) => (c as unknown as [string, { poster?: unknown }])[0])).toEqual(['/outgoing/1-clip.mp4', '/bad.mp4', '/a.jpg']);
    expect(go.mock.calls.map((c) => (c as unknown as [string, { poster?: unknown }])[1].poster)).toEqual([poster, undefined, undefined]);
    expect(videoOf).toHaveBeenCalledTimes(2);
  });

  it('send a phone\'s file whose name tells no kind as the copy the import makes, a video with its frame', async () => {
    // The photo picker of a phone names a file by a number: the import's copy says it is a video, and takes its frame.
    const poster = { jpeg: 'AAAA', width: 960, height: 2142 };
    const uri = 'content://media/picker/0/com.android.providers.media.photopicker/media/1000012345';
    const photo = 'content://media/picker/0/com.android.providers.media.photopicker/media/1000012346';
    const videoOf = vi.fn(async (path: string) => (path === uri ? { path: '/outgoing/1-207.mp4', poster } : { path: '/outgoing/2-IMG_1.jpg', poster: null }));
    const picked = new Picked(async () => null, videoOf);
    picked.add([uri, photo, 'content://com.android.externalstorage.documents/document/primary%3ADownload%2Fa.pdf']);
    const go = vi.fn(async () => {});
    await picked.send(go, 'Просто видео в тему');
    const calls = go.mock.calls.map((c) => c as unknown as [string, { poster?: unknown }]);
    expect(calls.map((c) => c[0])).toEqual(['/outgoing/1-207.mp4', '/outgoing/2-IMG_1.jpg', 'content://com.android.externalstorage.documents/document/primary%3ADownload%2Fa.pdf']);
    expect(calls.map((c) => c[1].poster)).toEqual([poster, null, undefined]);
    expect(videoOf.mock.calls.map((c) => c[0])).toEqual([uri, photo]);
  });

  it('send a phone\'s file of no known kind as it is when it cannot be copied', async () => {
    const uri = 'content://media/picker/0/com.android.providers.media.photopicker/media/1000012345';
    const picked = new Picked(async () => null, async () => { throw new Error('gone'); });
    picked.add([uri]);
    const go = vi.fn(async () => {});
    await picked.send(go);
    expect(go.mock.calls.map((c) => (c as unknown as [string])[0])).toEqual([uri]);
  });

  it('wait again from the one refused', async () => {
    const picked = new Picked();
    picked.add(['/a.jpg', '/b.jpg', '/c.jpg']);
    let i = 0;
    await expect(picked.send(async () => { if (++i === 2) throw new Error('no host'); }, 'x')).rejects.toThrow('no host');
    expect(picked.files.map((f) => f.path)).toEqual(['/b.jpg', '/c.jpg']);
  });
});

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The tray shows a picture as a small square of itself: asked once for
// each path, the icon when there is none, let go with the tray.

import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ImportedFile } from './api';
import { Picked, pickedIcon } from './picked.svelte';

/** A preview that answers when told to. */
function deferred() {
  const waiting = new Map<string, { ok: (v: string | null) => void; fail: (e: unknown) => void }>();
  const previewOf = vi.fn((path: string) => new Promise<string | null>((ok, fail) => waiting.set(path, { ok, fail })));
  return { previewOf, waiting };
}

const settle = () => new Promise((r) => setTimeout(r, 0));

afterEach(() => vi.restoreAllMocks());

describe('the pictures of the tray', () => {
  it('are asked once for each path', async () => {
    const previewOf = vi.fn(async (p: string) => `data:image/jpeg;base64,${p.length}`);
    const picked = new Picked(previewOf);
    picked.add(['/a.jpg', '/b.png']);
    picked.add(['/a.jpg']);
    await settle();
    expect(previewOf.mock.calls.map((c) => c[0])).toEqual(['/a.jpg', '/b.png']);
    expect(picked.previews).toEqual({ '/a.jpg': 'data:image/jpeg;base64,6', '/b.png': 'data:image/jpeg;base64,6' });
    // One of the two of a path taken out: the other still shows it.
    picked.remove(picked.files[2].key);
    expect(picked.previews['/a.jpg']).toBeTruthy();
  });

  it('are absent while asked, and the icon shows', async () => {
    const { previewOf, waiting } = deferred();
    const picked = new Picked(previewOf);
    picked.add(['/a.jpg']);
    expect('/a.jpg' in picked.previews).toBe(false);
    waiting.get('/a.jpg')?.ok('data:x');
    await settle();
    expect(picked.previews['/a.jpg']).toBe('data:x');
  });

  it('fall back to the icon when there is none, a refusal or a picture the page cannot draw', async () => {
    const { previewOf, waiting } = deferred();
    const picked = new Picked(previewOf);
    picked.add(['/doc.pdf', '/broken.jpg', '/a.jpg']);
    waiting.get('/doc.pdf')?.ok(null);
    waiting.get('/broken.jpg')?.fail(new Error('err.io'));
    waiting.get('/a.jpg')?.ok('data:a');
    await settle();
    expect(picked.previews).toEqual({ '/doc.pdf': null, '/broken.jpg': null, '/a.jpg': 'data:a' });
    picked.failed('/a.jpg');
    expect(picked.previews['/a.jpg']).toBeNull();
    // Not asked again for that.
    expect(previewOf).toHaveBeenCalledTimes(3);
  });

  it('go when their file is taken out, and an answer late for it is let go', async () => {
    const revoke = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
    const { previewOf, waiting } = deferred();
    const picked = new Picked(previewOf);
    picked.add(['/a.jpg', '/b.jpg']);
    waiting.get('/a.jpg')?.ok('blob:a');
    await settle();
    picked.remove(picked.files[0].key);
    expect(picked.previews).toEqual({});
    expect(revoke).toHaveBeenCalledWith('blob:a');
    picked.remove(picked.files[0].key);
    waiting.get('/b.jpg')?.ok('blob:b');
    await settle();
    expect(picked.previews).toEqual({});
    expect(revoke).toHaveBeenCalledWith('blob:b');
    // Picked again: asked again.
    picked.add(['/a.jpg']);
    expect(previewOf).toHaveBeenCalledTimes(3);
  });

  it('go with the tray cleared', async () => {
    const picked = new Picked(async () => 'data:x');
    picked.add(['/a.jpg', '/b.jpg']);
    await settle();
    picked.clear();
    expect(picked.previews).toEqual({});
  });

  it('go once their files are sent, and stay with those that wait again', async () => {
    const previewOf = vi.fn(async (p: string) => `data:${p}`);
    const picked = new Picked(previewOf);
    picked.add(['/a.jpg', '/b.jpg', '/c.jpg']);
    await settle();
    let i = 0;
    await expect(picked.send(async () => { if (++i === 2) throw new Error('no host'); })).rejects.toThrow('no host');
    expect(Object.keys(picked.previews).sort()).toEqual(['/a.jpg', '/b.jpg', '/c.jpg']);
    await picked.send(async () => {});
    expect(picked.files).toEqual([]);
    expect(picked.previews).toEqual({});
    expect(previewOf).toHaveBeenCalledTimes(3);
  });
});

describe('a phone\'s file added to the tray', () => {
  const heic = 'content://media/external/images/media/41';
  const broken = 'content://media/external/images/media/42';
  const copies: Record<string, ImportedFile> = {
    [heic]: { path: '/outgoing/1-IMG_1.jpg', name: 'IMG_1.jpg', preview: 'data:image/jpeg;base64,AAAA' },
    [broken]: { path: '/outgoing/2-IMG_2.heic', name: 'IMG_2.heic', note: 'unsupported_format' },
  };

  it('is the module\'s copy before it is sent: a HEIC a JPEG with its picture, an unreadable one a file the tray tells of', async () => {
    const previewOf = vi.fn(async () => null);
    const importOf = vi.fn(async (path: string) => copies[path]);
    const picked = new Picked(previewOf, undefined, importOf);
    picked.add([heic, broken, '/disk/a.png']);
    await settle();
    expect(importOf.mock.calls).toEqual([[heic, false], [broken, false]]);
    expect(picked.files.map((f) => [f.path, f.name, f.note])).toEqual([
      ['/outgoing/1-IMG_1.jpg', 'IMG_1.jpg', undefined],
      ['/outgoing/2-IMG_2.heic', 'IMG_2.heic', 'unsupported_format'],
      ['/disk/a.png', 'a.png', undefined],
    ]);
    expect(picked.previews).toEqual({ '/outgoing/1-IMG_1.jpg': 'data:image/jpeg;base64,AAAA', '/outgoing/2-IMG_2.heic': null, '/disk/a.png': null });
    // The phone's files are not asked of the decoder of pictures again; a file of the disk is.
    expect(previewOf.mock.calls.map((c) => (c as unknown[])[0])).toEqual(['/disk/a.png']);
    expect(picked.unsupported).toEqual(['IMG_2.heic']);

    const go = vi.fn(async () => {});
    await picked.send(go);
    expect(go.mock.calls.map((c) => (c as unknown as [string])[0])).toEqual(['/outgoing/1-IMG_1.jpg', '/outgoing/2-IMG_2.heic', '/disk/a.png']);
    expect(picked.unsupported).toEqual([]);
  });

  it('goes by «Файл» as it is, and is sent only once its copy is made', async () => {
    let done: (f: ImportedFile) => void = () => {};
    const importOf = vi.fn((path: string) => new Promise<ImportedFile>((ok) => { done = ok; void path; }));
    const picked = new Picked(async () => null, undefined, importOf);
    picked.add([heic], { asFile: true });
    expect(importOf).toHaveBeenCalledWith(heic, true);
    const go = vi.fn(async () => {});
    const sent = picked.send(go);
    await settle();
    expect(go).not.toHaveBeenCalled();
    done({ path: '/outgoing/3-IMG_1.heic', name: 'IMG_1.heic' });
    await sent;
    expect(go.mock.calls).toEqual([['/outgoing/3-IMG_1.heic', { caption: undefined, fmt: undefined, batch: undefined, asFile: true }]]);
  });

  it('taken out while it is copied stays out; one that could not be copied waits as picked', async () => {
    let done: (f: ImportedFile) => void = () => {};
    const picked = new Picked(async () => 'data:x', undefined, vi.fn(() => new Promise<ImportedFile>((ok) => { done = ok; })));
    picked.add([heic]);
    picked.remove(picked.files[0].key);
    done(copies[heic]);
    await settle();
    expect(picked.files).toEqual([]);
    expect(picked.previews).toEqual({});

    const failing = new Picked(async () => null, undefined, async () => { throw new Error('err.io'); });
    failing.add([heic]);
    await settle();
    expect(failing.files.map((f) => f.path)).toEqual([heic]);
    expect(failing.previews).toEqual({ [heic]: null });
  });

  it('a video goes with the frame of its copy', async () => {
    const poster = { jpeg: 'AAAA', width: 1920, height: 1080 };
    const posterOf = vi.fn(async () => poster);
    const videoOf = vi.fn();
    const clip = 'content://media/external/video/media/7';
    const picked = new Picked(async () => null, videoOf, async () => ({ path: '/outgoing/4-clip.mp4', name: 'clip.mp4', url: 'http://127.0.0.1:1/v' }), posterOf);
    picked.add([clip]);
    const go = vi.fn(async () => {});
    await picked.send(go);
    expect(go.mock.calls.map((c) => c as unknown as [string, { poster?: unknown }]).map((c) => [c[0], c[1].poster])).toEqual([['/outgoing/4-clip.mp4', poster]]);
    expect(posterOf).toHaveBeenCalledWith('http://127.0.0.1:1/v');
    expect(videoOf).not.toHaveBeenCalled();
  });
});

describe('the icon of a file without a picture', () => {
  it('tells a picture, a video and any other file', () => {
    expect(pickedIcon('Снимок экрана.png')).toBe('image');
    expect(pickedIcon('IMG_1.HEIC')).toBe('image');
    expect(pickedIcon('clip.mp4')).toBe('video');
    expect(pickedIcon('notes.pdf')).toBe('file');
    expect(pickedIcon('1234')).toBe('file');
  });
});

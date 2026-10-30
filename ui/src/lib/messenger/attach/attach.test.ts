// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The registry of the sheet of the paperclip (seams/attach): which items a
// composer is shown, in what order, and what a stopped module leaves behind.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { attachActions, provideAttachAction, type AttachAction, type AttachContext, type AttachRegistration } from '../seams/attach.svelte';
import { provideMessengerAttach } from './builtin';
import { providePreviewAttach } from './previewItem';
import { dialogLibrary, openOptions } from './DialogLibrary';
import { ResultGate, finishPanel, runAction } from './resultGate';
import galleryPanel from './GalleryPanel.svelte?raw';
import builtinSource from './builtin.ts?raw';
import { mockLibrary, MOCK_PICKED } from './MockLibrary';
import { holdBack, resetBacks, type SheetBackEnv } from './sheetBack';
import buttonSource from '../media/AttachButton.svelte?raw';

const ctx = (over: Partial<AttachContext> = {}): AttachContext => ({
  surface: 'dm', chatId: 'c', accepts: ['files'], limits: { maxBytes: 1, maxCount: 1 }, ...over,
});
const item = (id: string, order: number, over: Partial<AttachAction> = {}): AttachAction => ({
  id, order, icon: 'file', label: 'msg_attach_file', produces: 'files', ...over,
});
const ids = (c = ctx()) => attachActions.for(c).map((r) => r.action.id);

const taken: (() => void)[] = [];
const give = (a: AttachAction) => { const stop = provideAttachAction(a); taken.push(stop); return stop; };
afterEach(() => { taken.splice(0).forEach((stop) => stop()); });

describe('the items of the sheet', () => {
  it('stand by `order`, whatever the order they were given in', () => {
    give(item('c', 30)); give(item('a', 10)); give(item('b', 20));
    expect(ids()).toEqual(['a', 'b', 'c']);
  });

  it('are shown only where the composer accepts what they produce', () => {
    give(item('files', 1)); give(item('where', 2, { produces: 'location' }));
    expect(ids()).toEqual(['files']);
    expect(ids(ctx({ accepts: ['files', 'location'] }))).toEqual(['files', 'where']);
  });

  it('hide when `available` says false, and stay when it says nothing', () => {
    give(item('always', 1)); give(item('never', 2, { available: () => false })); give(item('chats', 3, { available: (c) => c.surface === 'dm' }));
    expect(ids()).toEqual(['always', 'chats']);
    expect(ids(ctx({ surface: 'post' }))).toEqual(['always']);
  });

  it('give way to a later item of the same id, and return one at a time', () => {
    const stop = give(item('x', 1));
    give(item('x', 5));
    expect(attachActions.all.filter((a) => a.id === 'x')).toHaveLength(1);
    stop(); // the first one was replaced: taking it back leaves the later one
    expect(ids()).toEqual(['x']);
  });

  it('are taken back with the module that gave them', () => {
    const stop = provideMessengerAttach();
    expect(ids()).toEqual(['gallery', 'file']);
    stop();
    expect(ids()).toEqual([]);
  });
});

describe('the messenger\'s own items', () => {
  it('are the gallery and the file, and the clipboard only in a chat that has a picture on it', () => {
    const stop = provideMessengerAttach();
    const byId = (id: string) => attachActions.all.find((a) => a.id === id)!;
    expect(byId('gallery').panel).toBeTruthy();
    expect(byId('file').run).toBeTruthy();
    expect(byId('clipboard').available?.(ctx({ surface: 'channel' }))).toBe(false);
    stop();
  });

  it('are not listed by the button any more', () => {
    expect(buttonSource).not.toContain('kinds');
  });

  it('are joined by a test item that is one file and changes nothing else', async () => {
    const stop = providePreviewAttach();
    expect(ids()).toEqual(['preview']);
    const result = await attachActions.all[0].run?.(ctx());
    expect(result).toMatchObject({ type: 'files', original: false });
    stop();
  });
});

describe('the source of media', () => {
  it('says what it can do, so a panel does not ask which source it is', async () => {
    expect(dialogLibrary.capabilities).toEqual({ pickSystem: true, grid: false, capture: false, photoPicker: false });
    expect(mockLibrary.capabilities.grid).toBe(true);
    expect(await mockLibrary.pickSystem({ multiple: true, kind: 'any' })).toEqual(MOCK_PICKED);
    expect(await mockLibrary.access()).toBe('full');
  });
});

describe('the system back', () => {
  function env() {
    const calls: string[] = [];
    let here = false;
    let onpop: (() => void) | null = null;
    const e: SheetBackEnv = {
      push: () => { here = true; calls.push('push'); },
      stepped: () => here,
      back: () => calls.push('back'),
      events: {
        addEventListener: (_t, f) => { onpop = f as () => void; },
        removeEventListener: () => { onpop = null; },
      },
    };
    // A back leaves the place, then the page hears of it.
    return { e, calls, pop: () => { here = false; onpop?.(); } };
  }

  beforeEach(resetBacks);

  it('closes the sheet and does not go back a second time', () => {
    const { e, calls, pop } = env();
    let closed = 0;
    const release = holdBack(() => closed++, e);
    pop();
    release();
    expect(closed).toBe(1);
    expect(calls).toEqual(['push']);
  });

  it('gives the place back when the sheet is closed another way', () => {
    const { e, calls } = env();
    holdBack(() => {}, e)();
    expect(calls).toEqual(['push', 'back']);
  });

  it('takes a back for the newest place only, and gives two places back one after the other', () => {
    let places: string[] = [];
    const listeners = new Set<() => void>();
    const calls: string[] = [];
    const env = (name: string): SheetBackEnv => ({
      push: () => { places = [...places, name]; },
      stepped: () => places.includes(name),
      back: () => { calls.push(`back ${places[places.length - 1]}`); },
      events: {
        addEventListener: (_t, f) => listeners.add(f as () => void),
        removeEventListener: (_t, f) => listeners.delete(f as () => void),
      },
    });
    const pop = () => { places = places.slice(0, -1); for (const f of [...listeners]) f(); };
    const closed: string[] = [];
    const releaseMode = holdBack(() => closed.push('mode'), env('mode'));
    const releaseSheet = holdBack(() => closed.push('sheet'), env('sheet'));
    pop();
    expect(closed).toEqual(['sheet']);
    releaseSheet();
    expect(calls).toEqual([]);

    const releaseAgain = holdBack(() => closed.push('sheet'), env('sheet'));
    releaseAgain();
    releaseMode();
    // The second back waits until the first has arrived.
    expect(calls).toEqual(['back sheet']);
    pop();
    expect(calls).toEqual(['back sheet', 'back mode']);
    pop();
    expect(closed).toEqual(['sheet']);
  });

  it('gives places back from the top down even when the lower one is let go first, and the next back still closes', () => {
    let places: string[] = ['chat'];
    const listeners = new Set<() => void>();
    const calls: string[] = [];
    const env = (name: string): SheetBackEnv => ({
      push: () => { places = [...places, name]; },
      stepped: () => places.includes(name),
      back: () => { calls.push(`back ${places[places.length - 1]}`); },
      events: {
        addEventListener: (_t, f) => listeners.add(f as () => void),
        removeEventListener: (_t, f) => listeners.delete(f as () => void),
      },
    });
    const pop = () => { places = places.slice(0, -1); for (const f of [...listeners]) f(); };
    const closed: string[] = [];
    // A forward sent from the mode of choosing closes both: the mode is let go before its sheet.
    const releaseMode = holdBack(() => closed.push('mode'), env('mode'));
    const releaseSheet = holdBack(() => closed.push('sheet'), env('sheet'));
    releaseMode();
    expect(calls).toEqual([]);
    releaseSheet();
    expect(calls).toEqual(['back sheet']);
    pop();
    expect(calls).toEqual(['back sheet', 'back mode']);
    pop();
    expect(places).toEqual(['chat']);
    expect(listeners.size).toBe(0);

    // Again and again: nothing is left behind, and a new mode closes on the first back.
    for (let i = 0; i < 2; i++) {
      const m = holdBack(() => closed.push('mode'), env('mode'));
      const s = holdBack(() => closed.push('sheet'), env('sheet'));
      m(); s(); pop(); pop();
    }
    expect(places).toEqual(['chat']);
    holdBack(() => closed.push('mode'), env('mode'));
    pop();
    expect(closed).toEqual(['mode']);
  });

  it('gives back a place let go under one the user then pops', () => {
    let places: string[] = ['chat'];
    const listeners = new Set<() => void>();
    const calls: string[] = [];
    const env = (name: string): SheetBackEnv => ({
      push: () => { places = [...places, name]; },
      stepped: () => places.includes(name),
      back: () => { calls.push(`back ${places[places.length - 1]}`); },
      events: {
        addEventListener: (_t, f) => listeners.add(f as () => void),
        removeEventListener: (_t, f) => listeners.delete(f as () => void),
      },
    });
    const pop = () => { places = places.slice(0, -1); for (const f of [...listeners]) f(); };
    const closed: string[] = [];
    const releaseMode = holdBack(() => closed.push('mode'), env('mode'));
    holdBack(() => closed.push('sheet'), env('sheet'));
    releaseMode();
    pop(); // the system back closes the sheet
    expect(closed).toEqual(['sheet']);
    expect(calls).toEqual(['back mode']);
    pop();
    expect(places).toEqual(['chat']);
  });

  it('does not go back when the page has already left that place', () => {
    const { e, calls } = env();
    const release = holdBack(() => {}, e);
    e.stepped = () => false;
    release();
    expect(calls).toEqual(['push']);
  });
});

describe('the options of the system chooser', () => {
  it('filter photos and videos and let a file be any file', () => {
    const media = openOptions(true, 'media');
    expect(media.filters?.[0].extensions).toEqual(expect.arrayContaining(['jpg', 'png', 'mp4', 'mov']));
    expect(media.filters?.[0].extensions).not.toContain('pdf');
    expect(openOptions(true, 'any')).toEqual({ multiple: true, directory: false, filters: undefined });
    expect(openOptions(false, 'image').filters?.[0].extensions).not.toContain('mp4');
  });

  it('are asked by the gallery for media, by the file for any file', () => {
    expect(galleryPanel).toContain("kind: 'media'");
    expect(builtinSource).toContain("kind: 'any'");
    expect(builtinSource).not.toContain("kind: 'media'");
  });

  it('reach plugin-dialog `open` as given for photo or video, file and (no chooser) clipboard', async () => {
    const calls: unknown[] = [];
    vi.doMock('@tauri-apps/plugin-dialog', () => ({ open: async (o: unknown) => { calls.push(o); return ['/p/a.png']; } }));
    const { dialogLibrary: lib } = await import('./DialogLibrary');
    await lib.pickSystem({ multiple: true, kind: 'media' });
    await lib.pickSystem({ multiple: true, kind: 'any' });
    expect((calls[0] as { filters?: unknown[] }).filters).toHaveLength(1);
    expect((calls[1] as { filters?: unknown[] }).filters).toBeUndefined();
    // The clipboard does not use the chooser at all.
    expect(builtinSource.split("id: 'clipboard'")[1]).not.toContain('pickSystem');
    vi.doUnmock('@tauri-apps/plugin-dialog');
  });
});

describe('who may hand a result to the composer', () => {
  // The sheet alone: every registration stands here; the registry is the next block.
  const act = (id: string): AttachRegistration => ({ action: { id, order: 1, icon: 'x', label: 'msg_attach_title', produces: 'files' } });
  const gallery = act('gallery'); const file = act('file');
  const opened = (chat = 'c1') => { const g = new ResultGate(() => true); g.open(chat); g.show('gallery'); return g; };

  it('takes the first answer of a panel and no second', () => {
    const g = opened();
    const t = g.panel(gallery);
    expect(g.take(t)).toBe(true);
    expect(g.take(t)).toBe(false);
  });

  it('drops the answer of a panel after the sheet was closed, and after it was opened again', () => {
    const g = opened();
    const t = g.panel(gallery);
    g.close();
    expect(g.take(t)).toBe(false);
    g.open('c1'); g.show('gallery');
    expect(g.take(t)).toBe(false);
    expect(g.take(g.panel(gallery))).toBe(true);
  });

  it('drops the answer of a panel whose tab was left, even if the tab is shown again', () => {
    const g = opened();
    const t = g.panel(gallery);
    g.show('map');
    expect(g.take(t)).toBe(false);
    g.show('gallery');
    expect(g.take(t)).toBe(false);
  });

  it('drops the answer when the chat changed, for a panel and for an item that runs', () => {
    const g = opened('c1');
    const p = g.panel(gallery); const r = g.run(file);
    g.close();
    g.moveTo('c2');
    expect(g.take(p)).toBe(false);
    expect(g.take(r)).toBe(false);
  });

  it('lets an item that runs answer after the sheet closed, once, but not after a new opening or the end of the composer', () => {
    const g = opened();
    const r = g.run(file);
    g.close();
    expect(g.take(r)).toBe(true);
    expect(g.take(r)).toBe(false);
    const r2 = g.run(file); g.open('c1');
    expect(g.take(r2)).toBe(false);
    const r3 = g.run(file); g.destroy();
    expect(g.take(r3)).toBe(false);
  });

  it('does not take the answer of a panel when the sheet is gone with the composer', () => {
    const g = opened();
    const t = g.panel(gallery);
    g.destroy();
    expect(g.take(t)).toBe(false);
  });
});

describe('who may hand a result to the composer, through the registry', () => {
  const files = { type: 'files' as const, paths: ['/a.jpg'], original: false };
  const mk = (id: string, run?: AttachAction['run']): AttachAction => ({ id, order: 1, icon: 'x', label: 'msg_attach_title', produces: 'files', run });
  // As the sheet does: the gate asks the registry, the registration is the one the sheet was shown.
  const sheet = (tab: string) => { const g = new ResultGate((r) => attachActions.alive(r)); g.open('c1'); g.show(tab); return g; };
  const regOf = (id: string) => attachActions.for(ctx()).find((r) => r.action.id === id)!;

  // What happens to the item between its start and its answer; `results`: how many answers the composer takes.
  const modes: { name: string; results: number; meanwhile: (a: AttachAction, stop: () => void) => void }[] = [
    { name: 'the registration stands', results: 1, meanwhile: () => {} },
    { name: 'it was taken back', results: 0, meanwhile: (_a, stop) => stop() },
    { name: 'it was taken back and another object of the same id given', results: 0, meanwhile: (a, stop) => { stop(); give(mk(a.id)); } },
    { name: 'it was taken back and the same object given again', results: 0, meanwhile: (a, stop) => { stop(); give(a); } },
    { name: 'another object of the same id replaced it', results: 0, meanwhile: (a) => { give(mk(a.id)); } },
    { name: 'the same object was given again over it', results: 0, meanwhile: (a) => { give(a); } },
  ];

  describe.each(modes)('when $name', ({ results, meanwhile }) => {
    it('the answer of a run', async () => {
      let finish!: (r: typeof files) => void;
      const a = mk('run', () => new Promise((res) => { finish = res; }));
      const stop = give(a);
      const s = { onresult: vi.fn(), onerror: vi.fn() };
      const p = runAction(sheet('x'), regOf('run'), ctx(), s);
      meanwhile(a, stop);
      finish(files); await p;
      expect(s.onresult).toHaveBeenCalledTimes(results);
      expect(s.onerror).not.toHaveBeenCalled();
    });

    it('the failure of a run', async () => {
      let fail!: (e: unknown) => void;
      const a = mk('bad', () => new Promise((_, rej) => { fail = rej; }));
      const stop = give(a);
      const s = { onresult: vi.fn(), onerror: vi.fn() };
      const p = runAction(sheet('x'), regOf('bad'), ctx(), s);
      meanwhile(a, stop);
      fail(new Error('x')); await p;
      expect(s.onerror).toHaveBeenCalledTimes(results);
      expect(s.onresult).not.toHaveBeenCalled();
    });

    it('the `done` of a panel', () => {
      const a = mk('panel');
      const stop = give(a);
      const g = sheet('panel'); const t = g.panel(regOf('panel'));
      const s = { onresult: vi.fn(), onerror: vi.fn() };
      meanwhile(a, stop);
      expect(finishPanel(g, t, files, s)).toBe(results === 1);
      expect(s.onresult).toHaveBeenCalledTimes(results);
    });
  });

  it('gives a new registration of the same object its own results, and the old way back takes only the old one', async () => {
    const a = mk('again', () => Promise.resolve(files));
    const stop = give(a);
    stop(); give(a);
    stop(); // the old way back, called again
    expect(ids()).toEqual(['again']);
    const s = { onresult: vi.fn(), onerror: vi.fn() };
    await runAction(sheet('x'), regOf('again'), ctx(), s);
    expect(s.onresult).toHaveBeenCalledTimes(1);
  });
});

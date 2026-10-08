// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The frames of a call as the channel carries them: the header and the
// planes are read as the runtime packs them (crates/messenger-app
// commands/calls.rs `pack`), and a turned frame is stood upright by the
// texture coordinates the renderer draws with.

import { describe, expect, it } from 'vitest';
import { FpsMeter, FrameAcks, FrameRenderer, HEADER_BYTES, WINDOW_CORNER, WINDOW_MARGIN, chromaSize, clampShift, fitFor, frameSeq, mirrorsLocal, parseFrame, quadCoords, shownSize, sourcePoint, testPattern } from './video';

/** A frame as the runtime packs it. */
function packed(width: number, height: number, rotation: number, seq: number, fill = 7): Uint8Array {
  const [cw, ch] = chromaSize(width, height);
  const out = new Uint8Array(HEADER_BYTES + width * height + 2 * cw * ch).fill(fill);
  const head = new DataView(out.buffer);
  head.setUint32(0, width, true);
  head.setUint32(4, height, true);
  head.setUint32(8, rotation, true);
  head.setUint32(12, seq, true);
  head.setBigInt64(16, 1_000_001n, true);
  return out;
}

describe('a frame of the channel', () => {
  it('is read behind its header, odd sizes too', () => {
    const f = parseFrame(packed(5, 3, 90, 4).buffer as ArrayBuffer);
    expect(f).not.toBe('end');
    expect(f).not.toBeNull();
    const frame = f as Exclude<typeof f, 'end' | null>;
    expect([frame.width, frame.height, frame.rotation, frame.seq]).toEqual([5, 3, 90, 4]);
    expect(frame.y.length).toBe(15);
    expect(frame.u.length).toBe(6);
    expect(frame.v.length).toBe(6);
    expect(shownSize(frame)).toEqual({ width: 3, height: 5 });
  });

  it('ends with an empty header, and what is not a frame is dropped', () => {
    expect(parseFrame(new ArrayBuffer(HEADER_BYTES))).toBe('end');
    expect(parseFrame(packed(4, 2, 0, 1).subarray(0, HEADER_BYTES + 3))).toBeNull();
    expect(parseFrame(packed(4, 2, 45, 1))).toBeNull();
    expect(parseFrame(new ArrayBuffer(10))).toBeNull();
    expect(parseFrame(Array.from(packed(2, 2, 0, 9))), 'a channel that gives numbers').toMatchObject({ width: 2, seq: 9 });
  });
});

describe('a turned frame', () => {
  /** The luma of a frame at a point of the shown picture, drawn as the renderer draws it. */
  function sample(buf: ArrayBuffer, x: number, y: number, mirror: boolean): number {
    const f = parseFrame(buf);
    if (!f || f === 'end') throw new Error('no frame');
    const [u, v] = sourcePoint(x, y, f.rotation, mirror);
    const i = Math.min(f.width - 1, Math.floor(u * f.width));
    const j = Math.min(f.height - 1, Math.floor(v * f.height));
    return f.y[j * f.width + i];
  }

  it('stands upright: every rotation shows what the unturned picture shows', () => {
    const upright = testPattern(64, 36, 3, 1);
    for (const rotation of [90, 180, 270]) {
      const turned = testPattern(64, 36, 3, 1, rotation);
      const f = parseFrame(turned) as Exclude<ReturnType<typeof parseFrame>, 'end' | null>;
      expect(shownSize(f), `${rotation}`).toEqual({ width: 64, height: 36 });
      for (let y = 0; y < 36; y += 5) {
        for (let x = 0; x < 64; x += 7) {
          const at = [(x + 0.5) / 64, (y + 0.5) / 36] as const;
          expect(sample(turned, at[0], at[1], false), `${rotation} at ${x},${y}`).toBe(sample(upright, at[0], at[1], false));
        }
      }
    }
  });

  it('is mirrored left to right', () => {
    const pic = testPattern(64, 36, 0, 1, 270);
    expect(sample(pic, 0.1, 0.05, true)).toBe(sample(pic, 0.9, 0.05, false));
  });

  it('gives the quad its corners', () => {
    expect(Array.from(quadCoords(0, false))).toEqual([0, 1, 1, 1, 0, 0, 1, 0]);
    expect(Array.from(quadCoords(0, true))).toEqual([1, 1, 0, 1, 1, 0, 0, 0]);
  });
});

describe('the fit of a picture', () => {
  it('fills a box that stands as it does, and shows whole one that does not', () => {
    expect(fitFor({ width: 640, height: 360 }, { width: 460, height: 288 })).toBe('cover');
    expect(fitFor({ width: 640, height: 360 }, { width: 412, height: 880 })).toBe('contain');
    expect(fitFor({ width: 360, height: 640 }, { width: 412, height: 880 })).toBe('cover');
    expect(fitFor(null, { width: 10, height: 10 })).toBe('cover');
  });
});

describe('frames a second', () => {
  it('counts the last second', () => {
    const m = new FpsMeter();
    for (let i = 0; i < 30; i++) m.frame(1000 + i * 33);
    expect(m.fps(1990)).toBe(30);
    expect(m.fps(2500)).toBe(14);
    expect(m.fps(5000)).toBe(0);
  });
});

describe('my own picture', () => {
  it('is mirrored for a face, not for the back camera or a screen', () => {
    expect(mirrorsLocal({ video_screen: false })).toBe(true);
    expect(mirrorsLocal({ video_screen: false, camera: 'front' })).toBe(true);
    expect(mirrorsLocal({ video_screen: false, camera: '/dev/video0' })).toBe(true);
    expect(mirrorsLocal({ video_screen: false, camera: 'back' })).toBe(false);
    expect(mirrorsLocal({ video_screen: true, camera: 'front' })).toBe(false);
  });
});

describe('the place of the video window', () => {
  const win = { width: 435, height: 272 };
  /** Where the window stands on a page with a shift: left, top, right, bottom. */
  function edges(s: { dx: number; dy: number }, page: { width: number; height: number }) {
    const left = page.width - WINDOW_CORNER - win.width + s.dx;
    const top = page.height - WINDOW_CORNER - win.height + s.dy;
    return { left, top, right: left + win.width, bottom: top + win.height };
  }

  it('brings a place kept from a larger page back onto a smaller one', () => {
    // Dragged to the top left of 1920×1080, then shown on 1280×720.
    const kept = clampShift({ dx: -5000, dy: -5000 }, win, { width: 1920, height: 1080 });
    expect(edges(kept, { width: 1920, height: 1080 })).toMatchObject({ left: WINDOW_MARGIN, top: WINDOW_MARGIN });
    const small = { width: 1280, height: 720 };
    expect(edges(kept, small).right, 'off the page as kept').toBeLessThan(0);
    const e = edges(clampShift(kept, win, small), small);
    expect(e).toMatchObject({ left: WINDOW_MARGIN, top: WINDOW_MARGIN });
    expect(e.right).toBeLessThanOrEqual(small.width - WINDOW_MARGIN);
    expect(e.bottom).toBeLessThanOrEqual(small.height - WINDOW_MARGIN);
  });

  it('keeps a place that is on the page, and holds every side', () => {
    const page = { width: 1280, height: 720 };
    expect(clampShift({ dx: 0, dy: 0 }, win, page)).toEqual({ dx: 0, dy: 0 });
    expect(clampShift({ dx: -300, dy: -100 }, win, page)).toEqual({ dx: -300, dy: -100 });
    expect(edges(clampShift({ dx: 900, dy: 900 }, win, page), page)).toMatchObject({ right: page.width - WINDOW_MARGIN, bottom: page.height - WINDOW_MARGIN });
    // A page smaller than the window: its top left stays on it, where the bar's buttons start.
    expect(edges(clampShift({ dx: -900, dy: 900 }, win, { width: 300, height: 200 }), { width: 300, height: 200 })).toMatchObject({ left: WINDOW_MARGIN, top: WINDOW_MARGIN });
  });

  it('brings back a window at the left edge that the list of a group call widened to the left', () => {
    // GroupCallWindow: held by its right edge; the list makes it wider, so its left side moves left.
    const page = { width: 1280, height: 720 };
    const narrow = { width: 563, height: 435 };
    const wide = { width: 742, height: 435 };
    const at = clampShift({ dx: -5000, dy: 0 }, narrow, page);
    const left = (s: { dx: number }, w: { width: number }) => page.width - WINDOW_CORNER - w.width + s.dx;
    expect(left(at, narrow)).toBe(WINDOW_MARGIN);
    expect(left(at, wide), 'the list opened, the place as it was').toBeLessThan(0);
    const fitted = clampShift(at, wide, page);
    expect(left(fitted, wide)).toBe(WINDOW_MARGIN);
    expect(fitted.dy).toBe(at.dy);
  });
});

/**
 * A WebGL context that records its calls and can be lost: enough of one
 * for the renderer to set itself up, draw and let go.
 */
function fakeGl(opts: { lostAtBirth?: boolean } = {}) {
  const state = { lost: !!opts.lostAtBirth, calls: [] as string[] };
  const gl = new Proxy({} as Record<string, unknown>, {
    get(_, name) {
      if (typeof name !== 'string') return undefined;
      if (/^[A-Z0-9_]+$/.test(name)) return 1;
      switch (name) {
        case 'isContextLost': return () => state.lost;
        case 'getShaderParameter': case 'getProgramParameter': return () => true;
        case 'getAttribLocation': return () => 0;
        case 'getExtension':
          return (ext: string) => ext === 'WEBGL_lose_context' ? { loseContext: () => { state.calls.push('loseContext'); state.lost = true; } } : null;
      }
      return () => {
        state.calls.push(name);
        // A lost context makes nothing.
        return name.startsWith('create') && state.lost ? null : {};
      };
    },
  });
  return { gl, state };
}

/** A canvas that, as a real one, gives one kind of context for its life. */
class FakeCanvas {
  width = 2;
  height = 2;
  taken: string | null = null;
  listeners = new Map<string, Set<(e: Event) => void>>();
  constructor(private gl: unknown, private ctx2d: unknown = {}) {}
  getContext(kind: string) {
    if (this.taken && this.taken !== kind) return null;
    const c = kind === 'webgl' ? this.gl : this.ctx2d;
    if (c) this.taken = kind;
    return c;
  }
  addEventListener(type: string, f: (e: Event) => void) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type)!.add(f);
  }
  removeEventListener(type: string, f: (e: Event) => void) {
    this.listeners.get(type)?.delete(f);
  }
  listening() {
    return [...this.listeners.values()].reduce((n, s) => n + s.size, 0);
  }
  fire(type: string) {
    const e = { defaultPrevented: false, preventDefault() { this.defaultPrevented = true; } };
    this.listeners.get(type)?.forEach((f) => f(e as unknown as Event));
    return e;
  }
}

describe('the renderer', () => {
  const frame = () => parseFrame(packed(4, 2, 0, 1)) as Exclude<ReturnType<typeof parseFrame>, 'end' | null>;
  const count = (calls: string[], name: string) => calls.filter((c) => c === name).length;

  it('asks for a new canvas when WebGL came but cannot draw, and draws nothing meanwhile', () => {
    const { gl, state } = fakeGl({ lostAtBirth: true });
    const canvas = new FakeCanvas(gl);
    const r = new FrameRenderer(canvas as unknown as HTMLCanvasElement);
    // The canvas holds WebGL now: no 2D from it.
    expect(r.mode).toBe('none');
    expect(r.broken).toBe(true);
    expect(r.draw(frame(), false)).toBe(false);
    expect(canvas.listening(), 'its events let go').toBe(0);
    expect(state.calls).toContain('loseContext');
    // A new canvas without WebGL draws in 2D.
    const fresh = new FrameRenderer(new FakeCanvas(fakeGl().gl) as unknown as HTMLCanvasElement, { webgl: false });
    expect(fresh.mode).toBe('2d');
    expect(fresh.broken).toBe(false);
  });

  it('draws nothing while its context is lost, and draws again once it is restored', () => {
    const { gl, state } = fakeGl();
    const canvas = new FakeCanvas(gl);
    const r = new FrameRenderer(canvas as unknown as HTMLCanvasElement);
    expect(r.mode).toBe('webgl');
    expect(r.draw(frame(), true)).toBe(true);
    expect(count(state.calls, 'bufferSubData')).toBe(1);

    // Lost before the event: the context tells at once.
    state.lost = true;
    expect(r.draw(frame(), true)).toBe(false);
    const lost = canvas.fire('webglcontextlost');
    expect(lost.defaultPrevented, 'restoration asked for').toBe(true);
    expect(r.draw(frame(), true)).toBe(false);

    state.lost = false;
    canvas.fire('webglcontextrestored');
    expect(count(state.calls, 'createProgram'), 'set up anew').toBe(2);
    expect(r.mode).toBe('webgl');
    expect(r.draw(frame(), true)).toBe(true);
    expect(count(state.calls, 'bufferSubData'), 'the corners placed anew').toBe(2);
  });

  it('is broken when the restored context cannot be set up', () => {
    const { gl, state } = fakeGl();
    const canvas = new FakeCanvas(gl);
    const r = new FrameRenderer(canvas as unknown as HTMLCanvasElement);
    state.lost = true;
    canvas.fire('webglcontextlost');
    // Restored, but lost again before it is set up.
    canvas.fire('webglcontextrestored');
    expect(r.broken).toBe(true);
    expect(r.draw(frame(), false)).toBe(false);
  });

  it('lets go of the program, the shaders, the textures and the context', () => {
    const { gl, state } = fakeGl();
    const canvas = new FakeCanvas(gl);
    const r = new FrameRenderer(canvas as unknown as HTMLCanvasElement);
    r.destroy();
    expect(count(state.calls, 'deleteProgram')).toBe(1);
    expect(count(state.calls, 'deleteShader')).toBe(2);
    expect(count(state.calls, 'deleteTexture')).toBe(3);
    expect(count(state.calls, 'deleteBuffer')).toBe(2);
    expect(state.calls).toContain('loseContext');
    expect(canvas.listening()).toBe(0);
    expect(r.draw(frame(), false)).toBe(false);
  });
});

describe('the acknowledgements of the frames', () => {
  it('reads the seq of any header, a frame or not', () => {
    expect(frameSeq(packed(4, 2, 0, 41))).toBe(41);
    // Planes cut short: no frame, still a seq to acknowledge.
    const cut = packed(4, 2, 0, 9).slice(0, HEADER_BYTES + 3);
    expect(parseFrame(cut)).toBeNull();
    expect(frameSeq(cut)).toBe(9);
    // A strange rotation: the same.
    const odd = packed(4, 2, 45, 12);
    expect(parseFrame(odd)).toBeNull();
    expect(frameSeq(odd)).toBe(12);
    expect(frameSeq(testPattern(8, 4, 0xfffffffe, 1))).toBe(0xfffffffe);
    expect(frameSeq(new Uint8Array(15))).toBeNull();
  });

  it('keeps the seqs that came before the id and sends them in order once it is there', () => {
    const sent: [number, number][] = [];
    const acks = new FrameAcks((id, seq) => { sent.push([id, seq]); });
    acks.took(0);
    acks.took(1);
    expect(sent).toEqual([]);
    acks.ready(7);
    expect(sent).toEqual([[7, 0], [7, 1]]);
    acks.took(2);
    expect(sent).toEqual([[7, 0], [7, 1], [7, 2]]);
    // The id comes once.
    acks.ready(8);
    acks.took(3);
    expect(sent.at(-1)).toEqual([7, 3]);
  });

  it('sends at once when the id came first', () => {
    const sent: [number, number][] = [];
    const acks = new FrameAcks((id, seq) => { sent.push([id, seq]); });
    acks.ready(3);
    acks.took(5);
    expect(sent).toEqual([[3, 5]]);
  });

  it('sends nothing after close, nor what waited', () => {
    const sent: [number, number][] = [];
    const acks = new FrameAcks((id, seq) => { sent.push([id, seq]); });
    acks.took(0);
    acks.close();
    acks.ready(1);
    acks.took(1);
    expect(sent).toEqual([]);
  });

  it('lets go of what the send throws or rejects with', async () => {
    let calls = 0;
    const throwing = new FrameAcks(() => { calls += 1; throw new Error('gone'); });
    throwing.ready(1);
    expect(() => throwing.took(0)).not.toThrow();
    const rejecting = new FrameAcks(() => { calls += 1; return Promise.reject(new Error('gone')); });
    rejecting.took(0);
    rejecting.ready(1);
    rejecting.took(1);
    // An unhandled rejection would fail the run.
    await new Promise((r) => setTimeout(r, 0));
    expect(calls).toBe(3);
  });
});

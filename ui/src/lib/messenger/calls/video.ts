// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The video of a call on the page. The runtime sends the frames of a track
// on a channel (`messenger_call_video_subscribe`), one message a frame:
// a header of 24 bytes, little-endian (u32 width, u32 height, u32 rotation,
// u32 seq, i64 timestamp_us), then the I420 planes packed tight (Y of
// width×height, U and V of ⌈width/2⌉×⌈height/2⌉). A header with width and
// height 0 and no planes is the last message (internal/messenger-wire.md
// §10, "Видео").
//
// The planes go to WebGL as three one-byte textures and a shader turns them
// into RGB: a third of the bytes of RGBA through the IPC, and well under a
// millisecond of drawing (tmp/calls-spike). The rotation of a phone's
// camera and the mirror of my own face are made by the texture coordinates,
// so the canvas holds the picture as it stands; CSS `object-fit` scales it.
// Without WebGL a 2D canvas does the conversion in JS.

export const HEADER_BYTES = 24;

export interface I420Frame {
  width: number;
  height: number;
  /** Degrees clockwise that stand the picture upright: 0, 90, 180, 270. */
  rotation: number;
  seq: number;
  y: Uint8Array;
  u: Uint8Array;
  v: Uint8Array;
}

/** The width and height of the chroma planes. */
export function chromaSize(width: number, height: number): [number, number] {
  return [(width + 1) >> 1, (height + 1) >> 1];
}

/** The size a frame shows at: a quarter turn swaps the sides. */
export function shownSize(f: Pick<I420Frame, 'width' | 'height' | 'rotation'>): { width: number; height: number } {
  return f.rotation === 90 || f.rotation === 270 ? { width: f.height, height: f.width } : { width: f.width, height: f.height };
}

/**
 * A message of the channel: a frame, `'end'` for the last message, `null`
 * for what is not a frame (too short for its planes, a strange rotation).
 */
export function parseFrame(data: ArrayBuffer | Uint8Array | number[]): I420Frame | 'end' | null {
  const bytes = data instanceof Uint8Array ? data : data instanceof ArrayBuffer ? new Uint8Array(data) : Uint8Array.from(data);
  if (bytes.byteLength < HEADER_BYTES) return null;
  const view = new DataView(bytes.buffer, bytes.byteOffset, HEADER_BYTES);
  const width = view.getUint32(0, true);
  const height = view.getUint32(4, true);
  const rotation = view.getUint32(8, true);
  const seq = view.getUint32(12, true);
  if (width === 0 && height === 0) return 'end';
  if (rotation % 90 !== 0 || rotation >= 360) return null;
  const ys = width * height;
  const [cw, ch] = chromaSize(width, height);
  const cs = cw * ch;
  if (bytes.byteLength < HEADER_BYTES + ys + 2 * cs) return null;
  const at = bytes.byteOffset + HEADER_BYTES;
  return {
    width, height, rotation, seq,
    y: new Uint8Array(bytes.buffer, at, ys),
    u: new Uint8Array(bytes.buffer, at + ys, cs),
    v: new Uint8Array(bytes.buffer, at + ys + cs, cs),
  };
}

/**
 * The `seq` of a message of the channel, read from its header even when
 * the rest is not a frame (planes cut short, a strange rotation): the page
 * acknowledges that one too. `null` for one too short to carry it.
 */
export function frameSeq(data: ArrayBuffer | Uint8Array | number[]): number | null {
  const bytes = data instanceof Uint8Array ? data : data instanceof ArrayBuffer ? new Uint8Array(data) : Uint8Array.from(data);
  if (bytes.byteLength < 16) return null;
  return new DataView(bytes.buffer, bytes.byteOffset, 16).getUint32(12, true);
}

/**
 * The acknowledgements of one subscription (`messenger_call_video_ack`):
 * the runtime sends the next frame only once the page took the last one,
 * so every message but the last is acknowledged by its `seq`, drawn or
 * not. The subscription's id comes later than its first frame can: the
 * `seq`s that came before are kept and sent, in order, once it is there.
 * Nothing goes after `close`. `send` is not waited for; what it throws
 * or rejects with is let go (an id that is over already).
 */
export class FrameAcks {
  private id: number | null = null;
  private waiting: number[] = [];
  private closed = false;

  constructor(private readonly send: (id: number, seq: number) => unknown) {}

  /** A message `seq` was taken. */
  took(seq: number) {
    if (this.closed) return;
    if (this.id == null) this.waiting.push(seq);
    else this.fire(this.id, seq);
  }

  /** The subscription's id came: what waited goes now. */
  ready(id: number) {
    if (this.closed || this.id != null) return;
    this.id = id;
    const waiting = this.waiting;
    this.waiting = [];
    for (const seq of waiting) this.fire(id, seq);
  }

  close() {
    this.closed = true;
    this.waiting = [];
  }

  private fire(id: number, seq: number) {
    try {
      const r = this.send(id, seq);
      if (r instanceof Promise) r.catch(() => {});
    } catch { /* the runtime is gone: nothing to tell */ }
  }
}

/**
 * Where on the source picture a point of the shown one is, both as 0..1
 * from the top left: the picture turned `rotation` degrees clockwise, then
 * mirrored left to right when `mirror`.
 */
export function sourcePoint(x: number, y: number, rotation: number, mirror: boolean): [number, number] {
  const u = mirror ? 1 - x : x;
  switch (rotation) {
    case 90: return [y, 1 - u];
    case 180: return [1 - u, 1 - y];
    case 270: return [1 - y, u];
    default: return [u, y];
  }
}

/** The texture coordinates of the quad's corners (bottom left, bottom right, top left, top right). */
export function quadCoords(rotation: number, mirror: boolean): Float32Array {
  const corners: [number, number][] = [[0, 1], [1, 1], [0, 0], [1, 0]];
  return new Float32Array(corners.flatMap(([x, y]) => sourcePoint(x, y, rotation, mirror)));
}

const VERTEX = `attribute vec2 p; attribute vec2 c; varying vec2 t;
  void main() { t = c; gl_Position = vec4(p, 0.0, 1.0); }`;
// BT.601, limited range: what libwebrtc's I420 is.
const FRAGMENT = `precision mediump float; varying vec2 t; uniform sampler2D y, u, v;
  void main() {
    float Y = 1.1643 * (texture2D(y, t).r - 0.0625);
    float U = texture2D(u, t).r - 0.5;
    float V = texture2D(v, t).r - 0.5;
    gl_FragColor = vec4(Y + 1.5958 * V, Y - 0.39173 * U - 0.81290 * V, Y + 2.017 * U, 1.0);
  }`;

/**
 * Draws I420 frames on a canvas, upright and (for my own face) mirrored.
 *
 * A canvas gives one kind of context for its whole life: once WebGL was
 * taken from it, a 2D one cannot be. So a canvas whose WebGL came but cannot
 * draw (a shader refused, a context lost at birth when too many are open) is
 * `broken`: the tile makes a new canvas and draws on it with `webgl: false`.
 * A context lost later (the GPU process restarted, the app was away) draws
 * nothing until it is restored, and then it is set up anew.
 */
export class FrameRenderer {
  mode: 'webgl' | '2d' | 'none' = 'none';
  private gl: WebGLRenderingContext | null = null;
  private program: WebGLProgram | null = null;
  private shaders: WebGLShader[] = [];
  private quad: WebGLBuffer | null = null;
  private coords: WebGLBuffer | null = null;
  private textures: WebGLTexture[] = [];
  private lost = false;
  private ctx: CanvasRenderingContext2D | null = null;
  private scratch: HTMLCanvasElement | null = null;
  private image: ImageData | null = null;
  private placed = '';

  constructor(private canvas: HTMLCanvasElement, opts: { webgl?: boolean } = {}) {
    if (opts.webgl !== false && this.initGl()) {
      this.mode = 'webgl';
      return;
    }
    try {
      this.ctx = canvas.getContext('2d');
    } catch {
      this.ctx = null;
    }
    this.mode = this.ctx ? '2d' : 'none';
  }

  /** This canvas can show nothing (more): a new one is needed, without WebGL. */
  get broken(): boolean {
    return this.mode === 'none';
  }

  private onLost = (e: Event) => {
    // Asked for, so the browser restores the context when it can.
    e.preventDefault();
    this.lost = true;
    this.forgetGl();
  };

  private onRestored = () => {
    this.lost = false;
    this.forgetGl();
    // The restored context holds nothing of the old one: set up anew, or
    // the canvas is done with.
    if (!this.gl || !this.setupGl(this.gl)) this.mode = 'none';
  };

  private initGl(): boolean {
    let gl: WebGLRenderingContext | null = null;
    try {
      gl = this.canvas.getContext('webgl', { antialias: false, depth: false, preserveDrawingBuffer: false });
    } catch {
      return false;
    }
    if (!gl) return false;
    this.gl = gl;
    this.canvas.addEventListener('webglcontextlost', this.onLost);
    this.canvas.addEventListener('webglcontextrestored', this.onRestored);
    if (!gl.isContextLost() && this.setupGl(gl)) return true;
    this.releaseGl();
    return false;
  }

  /** The program, the buffers and the textures, on a fresh or restored context. */
  private setupGl(gl: WebGLRenderingContext): boolean {
    const shader = (type: number, src: string) => {
      // Null on a lost context.
      const s = gl.createShader(type);
      if (!s) throw new Error('shader');
      this.shaders.push(s);
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? 'shader');
      return s;
    };
    try {
      const prog = gl.createProgram();
      if (!prog) throw new Error('program');
      this.program = prog;
      gl.attachShader(prog, shader(gl.VERTEX_SHADER, VERTEX));
      gl.attachShader(prog, shader(gl.FRAGMENT_SHADER, FRAGMENT));
      gl.linkProgram(prog);
      if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error('link');
      gl.useProgram(prog);
      this.quad = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, this.quad);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
      const p = gl.getAttribLocation(prog, 'p');
      gl.enableVertexAttribArray(p);
      gl.vertexAttribPointer(p, 2, gl.FLOAT, false, 0, 0);
      this.coords = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, this.coords);
      gl.bufferData(gl.ARRAY_BUFFER, quadCoords(0, false), gl.DYNAMIC_DRAW);
      const c = gl.getAttribLocation(prog, 'c');
      gl.enableVertexAttribArray(c);
      gl.vertexAttribPointer(c, 2, gl.FLOAT, false, 0, 0);
      // Rows of an odd width are not 4-byte aligned.
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      ['y', 'u', 'v'].forEach((name, i) => {
        const tex = gl.createTexture();
        if (!tex) throw new Error('texture');
        this.textures.push(tex);
        gl.activeTexture(gl.TEXTURE0 + i);
        gl.bindTexture(gl.TEXTURE_2D, tex);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        gl.uniform1i(gl.getUniformLocation(prog, name), i);
      });
    } catch {
      this.freeGl(gl);
      return false;
    }
    return !gl.isContextLost();
  }

  /** Deletes what was made on the context (a lost one ignores it). */
  private freeGl(gl: WebGLRenderingContext) {
    this.textures.forEach((t) => gl.deleteTexture(t));
    if (this.coords) gl.deleteBuffer(this.coords);
    if (this.quad) gl.deleteBuffer(this.quad);
    if (this.program) gl.deleteProgram(this.program);
    this.shaders.forEach((s) => gl.deleteShader(s));
    this.forgetGl();
  }

  /** The objects of a lost context went with it. */
  private forgetGl() {
    this.textures = [];
    this.shaders = [];
    this.coords = null;
    this.quad = null;
    this.program = null;
    this.placed = '';
  }

  /** Lets the context go: its objects, its events, and its place among the few a page may hold. */
  private releaseGl() {
    const gl = this.gl;
    if (!gl) return;
    this.canvas.removeEventListener('webglcontextlost', this.onLost);
    this.canvas.removeEventListener('webglcontextrestored', this.onRestored);
    try {
      this.freeGl(gl);
      gl.getExtension('WEBGL_lose_context')?.loseContext();
    } catch { /* the canvas may be gone already */ }
    this.gl = null;
  }

  /**
   * Draws a frame; the canvas takes the size it shows at. False when nothing
   * was drawn (the context is lost, or there is none): the tile is then not
   * live, and the screen shows its placeholder rather than a black canvas.
   */
  draw(f: I420Frame, mirror: boolean): boolean {
    if (this.mode === 'none') return false;
    const gl = this.gl;
    if (gl && (this.lost || gl.isContextLost())) return false;
    if (!gl && !this.ctx) return false;
    const shown = shownSize(f);
    if (this.canvas.width !== shown.width || this.canvas.height !== shown.height) {
      this.canvas.width = shown.width;
      this.canvas.height = shown.height;
    }
    if (!gl) {
      this.draw2d(f, mirror, shown);
      return true;
    }
    this.drawGl(gl, f, mirror, shown);
    // The loss comes as an event later; the context knows at once.
    return !gl.isContextLost();
  }

  private drawGl(gl: WebGLRenderingContext, f: I420Frame, mirror: boolean, shown: { width: number; height: number }) {
    const place = `${f.rotation}:${mirror}`;
    if (place !== this.placed) {
      gl.bindBuffer(gl.ARRAY_BUFFER, this.coords);
      gl.bufferSubData(gl.ARRAY_BUFFER, 0, quadCoords(f.rotation, mirror));
      this.placed = place;
    }
    gl.viewport(0, 0, shown.width, shown.height);
    const [cw, ch] = chromaSize(f.width, f.height);
    const planes: [Uint8Array, number, number][] = [[f.y, f.width, f.height], [f.u, cw, ch], [f.v, cw, ch]];
    planes.forEach(([data, w, h], i) => {
      gl.activeTexture(gl.TEXTURE0 + i);
      gl.bindTexture(gl.TEXTURE_2D, this.textures[i]);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.LUMINANCE, w, h, 0, gl.LUMINANCE, gl.UNSIGNED_BYTE, data);
    });
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  }

  private draw2d(f: I420Frame, mirror: boolean, shown: { width: number; height: number }) {
    const ctx = this.ctx;
    if (!ctx) return;
    const { width: w, height: h } = f;
    this.scratch ??= document.createElement('canvas');
    if (this.scratch.width !== w || this.scratch.height !== h || !this.image) {
      this.scratch.width = w;
      this.scratch.height = h;
      this.image = new ImageData(w, h);
    }
    const d = this.image.data;
    const cw = (w + 1) >> 1;
    for (let j = 0; j < h; j++) {
      for (let i = 0; i < w; i++) {
        const Y = 1.1643 * (f.y[j * w + i] - 16);
        const ci = (j >> 1) * cw + (i >> 1);
        const U = f.u[ci] - 128;
        const V = f.v[ci] - 128;
        const o = (j * w + i) * 4;
        d[o] = Y + 1.5958 * V;
        d[o + 1] = Y - 0.39173 * U - 0.8129 * V;
        d[o + 2] = Y + 2.017 * U;
        d[o + 3] = 255;
      }
    }
    this.scratch.getContext('2d')?.putImageData(this.image, 0, 0);
    ctx.save();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.translate(shown.width / 2, shown.height / 2);
    if (mirror) ctx.scale(-1, 1);
    ctx.rotate((f.rotation * Math.PI) / 180);
    ctx.drawImage(this.scratch, -w / 2, -h / 2);
    ctx.restore();
  }

  /**
   * Frees the program, the textures and the context itself (a page holds
   * only so many WebGL contexts); the canvas is not drawn on after this, the
   * tile makes a new one.
   */
  destroy() {
    this.releaseGl();
    this.mode = 'none';
    this.ctx = null;
    this.scratch = null;
    this.image = null;
  }
}

/** Frames a second, over the last second. */
export class FpsMeter {
  private stamps: number[] = [];

  frame(now = performance.now()) {
    this.stamps.push(now);
    this.trim(now);
  }

  fps(now = performance.now()): number {
    this.trim(now);
    return this.stamps.length;
  }

  private trim(now: number) {
    while (this.stamps.length && now - this.stamps[0] > 1000) this.stamps.shift();
  }
}

/**
 * No frame for this long: the signal of a video is taken as lost. A pause
 * of a second or two is ordinary (the node changes the layer of a
 * simulcast at a key frame, asks one at most once a second, starts a new
 * subscriber on the smallest layer; the way's bandwidth floats): the last
 * picture stays through it, and nothing on the screen moves.
 */
export const VIDEO_LOST_MS = 10_000;

/**
 * The signal of a video is lost at `now`: no frame since the last one
 * (`last`), or since the video was asked for (`since`: subscribed, or
 * turned on) when none came after, for `lostMs`. Times of one clock
 * (`performance.now()`); 0 is "never".
 */
export function signalLost(now: number, last: number, since: number, lostMs = VIDEO_LOST_MS): boolean {
  return now - Math.max(last, since) >= lostMs;
}

/**
 * What a tile of video has to show, as its frames come (VideoTile):
 * `live`, a picture (a frame was drawn since the video was asked for and
 * its stream did not end; a pause keeps it, the last picture stays);
 * `lost`, no frame for `lostMs` while the video is on, or the stream ended.
 * `on` is the word of the call (my camera, the peer's in a call of two):
 * while off nothing is held and no frame is to be drawn. Times of one
 * clock (`performance.now()`).
 */
export class PictureWatch {
  live = false;
  lost = false;
  private on = true;
  private last = 0;
  private since: number;
  private ended = false;

  constructor(now: number, private readonly lostMs = VIDEO_LOST_MS) {
    this.since = now;
  }

  /** Asked for anew (a new subscription): nothing is held, the frames are waited for from `now`. */
  restart(now: number) {
    this.since = now;
    this.last = 0;
    this.ended = false;
    this.live = false;
    this.lost = false;
  }

  /** The call says the video is on or off. Off: the picture is let go; on again: only a frame after `now` shows. */
  turn(on: boolean, now: number) {
    if (on === this.on) return;
    this.on = on;
    this.lost = false;
    if (on) {
      this.since = now;
      this.last = 0;
    } else {
      this.live = false;
    }
  }

  /** A frame is to be drawn: not while the call says the video is off. */
  get wanted(): boolean {
    return this.on;
  }

  /** A frame was drawn at `now`. */
  frame(now: number) {
    this.last = now;
    this.ended = false;
    this.live = true;
    this.lost = false;
  }

  /** The stream ended (its last message): no picture, lost. */
  end() {
    this.last = 0;
    this.ended = true;
    this.live = false;
    this.lost = true;
  }

  /** The clock moved on to `now`. */
  tick(now: number) {
    this.lost = this.ended || (this.on && signalLost(now, this.last, this.since, this.lostMs));
  }
}

/**
 * How a picture fills its box: whole (`contain`) when the box and the
 * picture stand differently (a landscape screen on an upright phone),
 * cropped to fill it (`cover`) when they are about alike.
 */
export function fitFor(picture: { width: number; height: number } | null, box: { width: number; height: number }): 'cover' | 'contain' {
  if (!picture || !picture.width || !picture.height || !box.width || !box.height) return 'cover';
  const a = picture.width / picture.height;
  const b = box.width / box.height;
  return Math.max(a, b) / Math.min(a, b) > 1.4 ? 'contain' : 'cover';
}

/**
 * Whether my own picture is shown mirrored: a face is, as in a mirror (a
 * computer's camera, a phone's front one); what a phone's back camera sees
 * is the world, and a screen is a page: neither is. The peer always gets
 * the picture unmirrored.
 */
export function mirrorsLocal(call: { video_screen: boolean; camera?: string }): boolean {
  return !call.video_screen && call.camera !== 'back';
}

/** The video window's corner: `right` and `bottom` of its CSS. */
export const WINDOW_CORNER = 16;
/** How close to the edge of the page the video window may go. */
export const WINDOW_MARGIN = 8;

/**
 * The shift (`translate`) of the video window from its corner held so that
 * the whole window stays on the page: a place kept from a larger page, or a
 * page made smaller, would leave it (and the only button that brings it
 * back) out of reach. A window larger than the page keeps its top left on it.
 */
export function clampShift(
  shift: { dx: number; dy: number },
  win: { width: number; height: number },
  page: { width: number; height: number },
): { dx: number; dy: number } {
  const axis = (d: number, size: number, room: number) => {
    const base = room - WINDOW_CORNER - size;
    const at = Math.max(WINDOW_MARGIN, Math.min(base + d, room - WINDOW_MARGIN - size));
    return at - base;
  };
  return { dx: axis(shift.dx, win.width, page.width), dy: axis(shift.dy, win.height, page.height) };
}

/**
 * A test picture as one frame of the channel, for the browser preview:
 * a field of `hue`, colour bars along the top, an arrow that points up when
 * the frame stands as it should, and a square that moves with `seq`.
 * `rotation` makes the buffer lie as a phone's camera gives it: turned so
 * that `rotation` degrees clockwise stand it upright.
 */
export function testPattern(width: number, height: number, seq: number, hue: number, rotation = 0): ArrayBuffer {
  // The upright picture is width × height; the buffer lies turned.
  const quarter = rotation === 90 || rotation === 270;
  const bw = quarter ? height : width;
  const bh = quarter ? width : height;
  const [cw, ch] = chromaSize(bw, bh);
  const out = new ArrayBuffer(HEADER_BYTES + bw * bh + 2 * cw * ch);
  const head = new DataView(out, 0, HEADER_BYTES);
  head.setUint32(0, bw, true);
  head.setUint32(4, bh, true);
  head.setUint32(8, rotation, true);
  head.setUint32(12, seq, true);
  head.setBigInt64(16, BigInt(Math.round(performance.now() * 1000)), true);
  const Y = new Uint8Array(out, HEADER_BYTES, bw * bh);
  const U = new Uint8Array(out, HEADER_BYTES + bw * bh, cw * ch);
  const V = new Uint8Array(out, HEADER_BYTES + bw * bh + cw * ch, cw * ch);

  const bars: [number, number, number][] = [[235, 128, 128], [210, 16, 146], [170, 166, 16], [145, 54, 34], [106, 202, 222], [81, 90, 240], [41, 240, 110]];
  const side = Math.round(Math.min(width, height) / 5);
  const travel = width - side;
  const t = (seq * 4) % (2 * travel);
  const sx = t < travel ? t : 2 * travel - t;
  const sy = Math.round(height * 0.55);
  const fu = Math.round(128 + 70 * Math.cos(hue));
  const fv = Math.round(128 + 70 * Math.sin(hue));
  const ax = width / 2;
  const ay = height * 0.2;
  const arrow = Math.min(width, height) * 0.16;

  // The value of the upright picture at (x, y): luma, U, V.
  const at = (x: number, y: number): [number, number, number] => {
    if (y < height * 0.1) return bars[Math.min(bars.length - 1, Math.floor((x / width) * bars.length))];
    if (x >= sx && x < sx + side && y >= sy && y < sy + side) return [235, 128, 128];
    const dy = y - ay;
    if (dy >= 0 && dy < arrow && Math.abs(x - ax) < dy * 0.6) return [235, 128, 128];
    if (dy >= arrow && dy < arrow * 2 && Math.abs(x - ax) < arrow * 0.18) return [235, 128, 128];
    return [60 + Math.round((y / height) * 60), fu, fv];
  };
  // Where the buffer's pixel (i, j) is in the upright picture.
  const upright = (i: number, j: number): [number, number] => {
    switch (rotation) {
      case 90: return [bh - 1 - j, i];
      case 180: return [bw - 1 - i, bh - 1 - j];
      case 270: return [j, bw - 1 - i];
      default: return [i, j];
    }
  };
  for (let j = 0; j < bh; j++) {
    for (let i = 0; i < bw; i++) {
      const [x, y] = upright(i, j);
      const px = at(x, y);
      Y[j * bw + i] = px[0];
      if ((i & 1) === 0 && (j & 1) === 0) {
        const c = (j >> 1) * cw + (i >> 1);
        U[c] = px[1];
        V[c] = px[2];
      }
    }
  }
  return out;
}

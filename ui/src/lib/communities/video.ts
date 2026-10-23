// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A video of a post or a comment before it plays: a frame with a play
// badge and its length, never the player's black box. The frame is the
// preview the sender took (`thumb` of its descriptor); a video sent
// without one (before senders took it) that is on this device already
// gets a frame taken here once, kept while the app runs.

/** `0:07`, `12:05`, `1:02:03`: the length of a video in its badge. */
export function videoLength(ms: number): string {
  const all = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(all / 3600);
  const m = Math.floor((all % 3600) / 60);
  const s = String(all % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

/** How large the frame of a video is drawn alone: its proportions within `side`×`side` (16:9 when unknown). */
export function frameBox(dim: [number, number] | undefined, side = 420): { width: number; ratio: string } {
  const [w, h] = dim && dim[0] > 0 && dim[1] > 0 ? dim : [16, 9];
  return { width: Math.round(Math.min(side, (side * w) / h)), ratio: `${w} / ${h}` };
}

/** A frame taken here: a picture to show and, as the video told it, its length. */
export interface Frame {
  src: string;
  durationMs: number | null;
}

/** The frames taken while the app runs, by the file; a video that gave none is not asked again. */
export class FrameCache {
  private kept = new Map<string, Frame | null>();
  private flights = new Map<string, Promise<Frame | null>>();

  constructor(private max = 64) {}

  /** What is known of a file: a frame, `null` when none can be taken, `undefined` when not tried. */
  peek(key: string): Frame | null | undefined {
    return this.kept.get(key);
  }

  /** The frame of a file, taken once for everyone who asks meanwhile. */
  take(key: string, grab: () => Promise<Frame | null>): Promise<Frame | null> {
    if (this.kept.has(key)) return Promise.resolve(this.kept.get(key) ?? null);
    let flight = this.flights.get(key);
    if (!flight) {
      flight = grab()
        .catch(() => null)
        .then((f) => {
          this.kept.set(key, f);
          for (const k of this.kept.keys()) {
            if (this.kept.size <= this.max) break;
            this.kept.delete(k);
          }
          return f;
        })
        .finally(() => this.flights.delete(key));
      this.flights.set(key, flight);
    }
    return flight;
  }
}

export const frames = new FrameCache();

/** The longer side of a frame taken here. */
const SIDE = 640;

/**
 * A frame of the video at `url` (the messenger's server of files, or a
 * `blob:`) and its length; `null` when the webview cannot read or decode
 * it in time — a computer without the decoder of its codec, for one.
 */
export function grabFrame(url: string, timeoutMs = 6000): Promise<Frame | null> {
  return new Promise((resolve) => {
    const video = document.createElement('video');
    let done = false;
    const finish = (f: Frame | null) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      video.removeAttribute('src');
      video.load();
      resolve(f);
    };
    const timer = setTimeout(() => finish(null), timeoutMs);
    video.muted = true;
    video.playsInline = true;
    video.preload = 'auto';
    // Read from another origin (the server on the loopback): the canvas stays readable.
    video.crossOrigin = 'anonymous';
    video.onerror = () => finish(null);
    // A little after the start, past a black first frame, as a chat's sender takes it.
    video.onloadedmetadata = () => { video.currentTime = Math.min(1, Math.max(0.1, (Number.isFinite(video.duration) ? video.duration : 0) / 10)); };
    video.onseeked = () => {
      try {
        const w = video.videoWidth;
        const h = video.videoHeight;
        if (!w || !h) return finish(null);
        const k = Math.min(1, SIDE / Math.max(w, h));
        const canvas = document.createElement('canvas');
        canvas.width = Math.max(1, Math.round(w * k));
        canvas.height = Math.max(1, Math.round(h * k));
        canvas.getContext('2d')?.drawImage(video, 0, 0, canvas.width, canvas.height);
        const src = canvas.toDataURL('image/jpeg', 0.8);
        const durationMs = Number.isFinite(video.duration) && video.duration > 0 ? Math.round(video.duration * 1000) : null;
        finish(src.startsWith('data:image/jpeg') ? { src, durationMs } : null);
      } catch {
        finish(null);
      }
    };
    video.src = url;
  });
}

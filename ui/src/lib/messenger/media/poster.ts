// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A frame of a video the composer sends: what the other side sees before
// it fetches the file. Taken here because the webview decodes video and the
// runtime does not; the runtime writes it anew as a small JPEG.

import type { MessengerPoster } from '../api';

/** The longer side of the frame handed over; the runtime makes it smaller still. */
const SIDE = 480;

/** Where in the video the frame is taken: a little after the start, past a black first frame. */
export function frameTime(duration: number): number {
  return Math.min(1, Math.max(0.1, (Number.isFinite(duration) ? duration : 0) / 10));
}

/** A frame as a picture the composer can show. */
export function posterUrl(p: MessengerPoster): string {
  return `data:image/jpeg;base64,${p.jpeg}`;
}

/**
 * A frame of the video the webview reads at `url` (the app's server on the
 * loopback, as `messenger_media_import` gave it); `null` when it cannot
 * read or draw it in time.
 */
export function videoPoster(url: string, timeoutMs = 5000): Promise<MessengerPoster | null> {
  return new Promise((resolve) => {
    const video = document.createElement('video');
    let done = false;
    const finish = (p: MessengerPoster | null) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      video.removeAttribute('src');
      video.load();
      resolve(p);
    };
    const timer = setTimeout(() => finish(null), timeoutMs);
    video.muted = true;
    video.playsInline = true;
    video.preload = 'auto';
    // The file comes from another origin: drawn without this, the canvas could not be read.
    video.crossOrigin = 'anonymous';
    video.onerror = () => finish(null);
    video.onloadedmetadata = () => { video.currentTime = frameTime(video.duration); };
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
        const url = canvas.toDataURL('image/jpeg', 0.8);
        const jpeg = url.startsWith('data:image/jpeg') ? url.slice(url.indexOf(',') + 1) : '';
        finish(jpeg ? { jpeg, width: w, height: h } : null);
      } catch {
        finish(null);
      }
    };
    video.src = url;
  });
}

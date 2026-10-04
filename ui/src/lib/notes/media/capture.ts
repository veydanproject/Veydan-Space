// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Recording engine on top of getUserMedia + MediaRecorder.

import { api } from '$lib/notes/api';
import { extForMime } from './kind';
import type { CaptureFile, MediaKind, MediaPrefs } from './types';

export type CaptureErrorCode = 'denied' | 'no-device' | 'busy' | 'unsupported';

export class CaptureError extends Error {
  constructor(public code: CaptureErrorCode, cause?: unknown) {
    super(code, { cause });
  }
}

const AUDIO_MIMES = ['audio/webm;codecs=opus', 'audio/webm', 'audio/mp4', 'audio/ogg;codecs=opus'];
const VIDEO_MIMES = ['video/webm;codecs=vp8,opus', 'video/webm;codecs=vp9,opus', 'video/webm', 'video/mp4'];

/** First recorder mime the webview supports, or null. */
export function pickMime(kind: MediaKind): string | null {
  if (typeof MediaRecorder === 'undefined') return null;
  const list = kind === 'audio' ? AUDIO_MIMES : VIDEO_MIMES;
  return list.find((m) => MediaRecorder.isTypeSupported(m)) ?? null;
}

export function captureSupported(): boolean {
  return typeof navigator !== 'undefined' && !!navigator.mediaDevices?.getUserMedia && typeof MediaRecorder !== 'undefined';
}

function constraints(kind: MediaKind, prefs: MediaPrefs): MediaStreamConstraints {
  if (kind === 'audio') return { audio: true };
  return {
    audio: true,
    video: { facingMode: prefs.facing, height: { ideal: prefs.videoHeight } },
  };
}

/** Opens mic (and camera for video); maps DOM errors to `CaptureError`. */
export async function openStream(kind: MediaKind, prefs: MediaPrefs): Promise<MediaStream> {
  if (!captureSupported() || !pickMime(kind)) throw new CaptureError('unsupported');
  try {
    await api.media.grantAccess();
    return await navigator.mediaDevices.getUserMedia(constraints(kind, prefs));
  } catch (e) {
    const name = (e as DOMException)?.name ?? '';
    if (name === 'NotAllowedError' || name === 'SecurityError') throw new CaptureError('denied', e);
    if (name === 'NotFoundError' || name === 'OverconstrainedError') throw new CaptureError('no-device', e);
    if (name === 'NotReadableError' || name === 'AbortError') throw new CaptureError('busy', e);
    throw new CaptureError('unsupported', e);
  }
}

export function closeStream(stream: MediaStream | null) {
  stream?.getTracks().forEach((t) => t.stop());
}

/** Camera facing of the first video track, or null for audio streams. */
export function streamFacing(stream: MediaStream): string | null {
  return stream.getVideoTracks()[0]?.getSettings().facingMode ?? null;
}

function fileName(kind: MediaKind, mime: string): string {
  const stamp = new Date().toISOString().slice(0, 19).replace('T', '_').replace(/:/g, '-');
  return `${kind}_${stamp}.${extForMime(mime, kind)}`;
}

/** One recording session over an open stream. */
export class Recorder {
  private rec: MediaRecorder;
  private chunks: Blob[] = [];
  private startedAt = 0;
  readonly mime: string;

  constructor(private stream: MediaStream, private kind: MediaKind, prefs: MediaPrefs) {
    this.mime = pickMime(kind) ?? '';
    const audioBitsPerSecond = prefs.audioKbps * 1000;
    const opts: MediaRecorderOptions = { mimeType: this.mime, audioBitsPerSecond };
    if (kind === 'video') opts.videoBitsPerSecond = videoBitrate(prefs.videoHeight);
    this.rec = new MediaRecorder(stream, opts);
    this.rec.ondataavailable = (e) => {
      if (e.data.size) this.chunks.push(e.data);
    };
  }

  start() {
    this.chunks = [];
    this.startedAt = performance.now();
    this.rec.start(1000);
  }

  /** Elapsed time since `start`. */
  elapsedMs(): number {
    return this.startedAt ? performance.now() - this.startedAt : 0;
  }

  /** Stops the recorder and returns the assembled file; the stream stays open. */
  stop(): Promise<CaptureFile> {
    return new Promise((resolve, reject) => {
      const durationMs = Math.round(this.elapsedMs());
      this.rec.onstop = () => {
        const blob = new Blob(this.chunks, { type: this.mime });
        resolve({ kind: this.kind, blob, mime: this.mime, name: fileName(this.kind, this.mime), durationMs });
      };
      this.rec.onerror = (e) => reject((e as ErrorEvent).error ?? new Error('recorder'));
      if (this.rec.state === 'inactive') this.rec.onstop(new Event('stop'));
      else this.rec.stop();
    });
  }

  /** Drops recorded data without producing a file. */
  cancel() {
    this.rec.onstop = null;
    if (this.rec.state !== 'inactive') this.rec.stop();
    this.chunks = [];
  }
}

function videoBitrate(height: number): number {
  if (height >= 1080) return 5_000_000;
  if (height >= 720) return 2_500_000;
  return 1_000_000;
}

/** Feeds 0..1 input level to `cb` until the returned function is called. */
export function levelMeter(stream: MediaStream, cb: (level: number) => void): () => void {
  if (typeof AudioContext === 'undefined' || !stream.getAudioTracks().length) return () => {};
  const ctx = new AudioContext();
  const src = ctx.createMediaStreamSource(stream);
  const analyser = ctx.createAnalyser();
  analyser.fftSize = 512;
  src.connect(analyser);
  const buf = new Uint8Array(analyser.fftSize);
  let raf = 0;
  const tick = () => {
    analyser.getByteTimeDomainData(buf);
    let sum = 0;
    for (const v of buf) {
      const d = (v - 128) / 128;
      sum += d * d;
    }
    cb(Math.min(1, Math.sqrt(sum / buf.length) * 3));
    raf = requestAnimationFrame(tick);
  };
  tick();
  return () => {
    cancelAnimationFrame(raf);
    src.disconnect();
    ctx.close().catch(() => {});
  };
}

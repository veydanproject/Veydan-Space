// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Recording from the microphone and the camera: getUserMedia +
// MediaRecorder. The host is only asked to let the webview answer the
// permission request; everything else is web platform.

import { messengerApi } from '../api';

export type CaptureKind = 'voice' | 'circle';
export type CaptureErrorCode = 'denied' | 'no_device' | 'busy' | 'unsupported';

export class CaptureError extends Error {
  constructor(public code: CaptureErrorCode, cause?: unknown) {
    super(code, { cause });
  }
}

const AUDIO = ['audio/webm;codecs=opus', 'audio/webm', 'audio/ogg;codecs=opus', 'audio/mp4'];
const VIDEO = ['video/webm;codecs=vp8,opus', 'video/webm;codecs=vp9,opus', 'video/webm', 'video/mp4'];

/** Longest recording, seconds. */
export const LIMIT_SECS: Record<CaptureKind, number> = { voice: 600, circle: 60 };
/** Bars in the outline sent with a voice message. */
export const WAVEFORM_BARS = 48;

export function pickMime(kind: CaptureKind): string | null {
  if (typeof MediaRecorder === 'undefined') return null;
  return (kind === 'voice' ? AUDIO : VIDEO).find((m) => MediaRecorder.isTypeSupported(m)) ?? null;
}

export function captureSupported(kind: CaptureKind): boolean {
  return typeof navigator !== 'undefined' && !!navigator.mediaDevices?.getUserMedia && pickMime(kind) !== null;
}

export async function openStream(kind: CaptureKind, facing: 'user' | 'environment' = 'user'): Promise<MediaStream> {
  if (!captureSupported(kind)) throw new CaptureError('unsupported');
  try {
    await messengerApi.media.grantAccess().catch(() => {});
    return await navigator.mediaDevices.getUserMedia(
      kind === 'voice'
        ? { audio: { echoCancellation: true, noiseSuppression: true } }
        : { audio: true, video: { facingMode: facing, width: { ideal: 480 }, height: { ideal: 480 }, aspectRatio: 1 } },
    );
  } catch (e) {
    const name = (e as DOMException)?.name ?? '';
    if (name === 'NotAllowedError' || name === 'SecurityError') throw new CaptureError('denied', e);
    if (name === 'NotFoundError' || name === 'OverconstrainedError') throw new CaptureError('no_device', e);
    if (name === 'NotReadableError' || name === 'AbortError') throw new CaptureError('busy', e);
    throw new CaptureError('unsupported', e);
  }
}

export function closeStream(stream: MediaStream | null) {
  stream?.getTracks().forEach((t) => t.stop());
}

/** Squeeze any number of level samples (0..1) into `bars` values 0..255. */
export function toWaveform(levels: number[], bars = WAVEFORM_BARS): number[] {
  if (levels.length === 0) return [];
  const out: number[] = [];
  for (let i = 0; i < bars; i++) {
    const from = Math.floor((i * levels.length) / bars);
    const to = Math.max(from + 1, Math.floor(((i + 1) * levels.length) / bars));
    let peak = 0;
    for (let j = from; j < to && j < levels.length; j++) peak = Math.max(peak, levels[j]);
    out.push(Math.max(0, Math.min(255, Math.round(peak * 255))));
  }
  return out;
}

export interface Captured {
  blob: Blob;
  mime: string;
  durationMs: number;
  waveform: number[];
}

/** One recording over an open stream, with a live level for the UI. */
export class Recorder {
  private rec: MediaRecorder;
  private chunks: Blob[] = [];
  private startedAt = 0;
  private levels: number[] = [];
  private stopMeter: () => void = () => {};
  readonly mime: string;

  constructor(stream: MediaStream, kind: CaptureKind, onlevel?: (level: number) => void) {
    this.mime = pickMime(kind) ?? '';
    this.rec = new MediaRecorder(stream, {
      mimeType: this.mime,
      audioBitsPerSecond: kind === 'voice' ? 32_000 : 48_000,
      ...(kind === 'circle' ? { videoBitsPerSecond: 700_000 } : {}),
    });
    this.rec.ondataavailable = (e) => { if (e.data.size) this.chunks.push(e.data); };
    this.stopMeter = meter(stream, (l) => { this.levels.push(l); onlevel?.(l); });
  }

  start() {
    this.chunks = [];
    this.levels = [];
    this.startedAt = performance.now();
    this.rec.start(500);
  }

  elapsedMs(): number {
    return this.startedAt ? performance.now() - this.startedAt : 0;
  }

  stop(): Promise<Captured> {
    return new Promise((resolve, reject) => {
      const durationMs = Math.round(this.elapsedMs());
      this.stopMeter();
      const done = () => resolve({
        blob: new Blob(this.chunks, { type: this.mime }),
        mime: this.mime,
        durationMs,
        waveform: toWaveform(this.levels),
      });
      this.rec.onstop = done;
      this.rec.onerror = (e) => reject((e as ErrorEvent).error ?? new Error('recorder'));
      if (this.rec.state === 'inactive') done();
      else this.rec.stop();
    });
  }

  cancel() {
    this.stopMeter();
    this.rec.onstop = null;
    if (this.rec.state !== 'inactive') this.rec.stop();
    this.chunks = [];
  }
}

/** Input level 0..1, about ten times a second. */
function meter(stream: MediaStream, cb: (level: number) => void): () => void {
  if (typeof AudioContext === 'undefined' || !stream.getAudioTracks().length) return () => {};
  const ctx = new AudioContext();
  const src = ctx.createMediaStreamSource(stream);
  const analyser = ctx.createAnalyser();
  analyser.fftSize = 512;
  src.connect(analyser);
  const buf = new Uint8Array(analyser.fftSize);
  const timer = setInterval(() => {
    analyser.getByteTimeDomainData(buf);
    let sum = 0;
    for (const v of buf) { const d = (v - 128) / 128; sum += d * d; }
    cb(Math.min(1, Math.sqrt(sum / buf.length) * 3));
  }, 100);
  return () => {
    clearInterval(timer);
    src.disconnect();
    ctx.close().catch(() => {});
  };
}

/** `m:ss` */
export function clockOf(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

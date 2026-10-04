// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// File extension to media kind / mime mapping.

import type { MediaKind } from './types';

export const AUDIO_EXTS = ['m4a', 'mp3', 'aac', 'wav', 'ogg', 'opus', 'weba', 'flac'];
export const VIDEO_EXTS = ['mp4', 'webm', 'mov', 'mkv', 'm4v'];

const MIME: Record<string, string> = {
  m4a: 'audio/mp4',
  mp3: 'audio/mpeg',
  aac: 'audio/aac',
  wav: 'audio/wav',
  ogg: 'audio/ogg',
  opus: 'audio/ogg',
  weba: 'audio/webm',
  flac: 'audio/flac',
  mp4: 'video/mp4',
  webm: 'video/webm',
  mov: 'video/quicktime',
  mkv: 'video/x-matroska',
  m4v: 'video/mp4',
};

function extOf(name: string): string {
  return name.split('.').pop()?.toLowerCase() ?? '';
}

export function mediaKindOf(name: string): MediaKind | null {
  const ext = extOf(name);
  if (AUDIO_EXTS.includes(ext)) return 'audio';
  if (VIDEO_EXTS.includes(ext)) return 'video';
  return null;
}

export function mediaMimeOf(name: string): string | null {
  return MIME[extOf(name)] ?? null;
}

/** Extension for a recorder mime type such as `audio/webm;codecs=opus`. */
export function extForMime(mime: string, kind: MediaKind): string {
  const base = mime.split(';')[0].trim();
  if (base === 'audio/webm') return 'weba';
  if (base === 'video/webm') return 'webm';
  if (base === 'audio/mp4') return 'm4a';
  if (base === 'video/mp4') return 'mp4';
  if (base === 'audio/ogg') return 'ogg';
  return kind === 'audio' ? 'weba' : 'webm';
}

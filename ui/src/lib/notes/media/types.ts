// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Shared types of the audio/video capture module. The module knows nothing
// about notes: the host maps its own files to `MediaItem` and stores
// `CaptureFile` results wherever it wants.

export type MediaKind = 'audio' | 'video';

/** A stored recording or attached media file shown in the sheet list. */
export interface MediaItem {
  name: string;
  size: number;
  kind: MediaKind;
  /** False while the file exists only remotely and cannot be played yet. */
  present: boolean;
}

/** Result of a finished recording, handed to the host for storage. */
export interface CaptureFile {
  kind: MediaKind;
  blob: Blob;
  mime: string;
  name: string;
  durationMs: number;
}

export type AudioKbps = 32 | 64 | 128;
export type VideoHeight = 480 | 720 | 1080;
export type CameraFacing = 'user' | 'environment';

/** Quick settings kept in localStorage. */
export interface MediaPrefs {
  audioKbps: AudioKbps;
  videoHeight: VideoHeight;
  facing: CameraFacing;
}

export const AUDIO_KBPS: AudioKbps[] = [32, 64, 128];
export const VIDEO_HEIGHTS: VideoHeight[] = [480, 720, 1080];

export const DEFAULT_MEDIA_PREFS: MediaPrefs = { audioKbps: 64, videoHeight: 720, facing: 'environment' };

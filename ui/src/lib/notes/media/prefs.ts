// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Quick settings persisted per host under `storageKey`.

import { AUDIO_KBPS, DEFAULT_MEDIA_PREFS, VIDEO_HEIGHTS, type MediaPrefs } from './types';

export const DEFAULT_STORAGE_KEY = 'veydan.media';

export function loadPrefs(key = DEFAULT_STORAGE_KEY): MediaPrefs {
  if (typeof localStorage === 'undefined') return { ...DEFAULT_MEDIA_PREFS };
  try {
    const raw = JSON.parse(localStorage.getItem(key) ?? '{}') as Partial<MediaPrefs>;
    return {
      audioKbps: AUDIO_KBPS.includes(raw.audioKbps!) ? raw.audioKbps! : DEFAULT_MEDIA_PREFS.audioKbps,
      videoHeight: VIDEO_HEIGHTS.includes(raw.videoHeight!) ? raw.videoHeight! : DEFAULT_MEDIA_PREFS.videoHeight,
      facing: raw.facing === 'user' || raw.facing === 'environment' ? raw.facing : DEFAULT_MEDIA_PREFS.facing,
    };
  } catch {
    return { ...DEFAULT_MEDIA_PREFS };
  }
}

export function savePrefs(prefs: MediaPrefs, key = DEFAULT_STORAGE_KEY) {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(key, JSON.stringify(prefs));
}

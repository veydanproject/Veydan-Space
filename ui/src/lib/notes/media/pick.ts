// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Native picker filtered to one media kind. Returns paths (desktop) or
// `content://` URIs (Android); storing them is up to the host.

import { AUDIO_EXTS, VIDEO_EXTS } from './kind';
import type { MediaKind } from './types';

export async function pickMediaFiles(kind: MediaKind): Promise<string[]> {
  const { open } = await import('@tauri-apps/plugin-dialog');
  const picked = await open({
    multiple: true,
    filters: [{ name: kind === 'audio' ? 'Audio' : 'Video', extensions: kind === 'audio' ? AUDIO_EXTS : VIDEO_EXTS }],
  });
  if (!picked) return [];
  return Array.isArray(picked) ? picked : [picked];
}

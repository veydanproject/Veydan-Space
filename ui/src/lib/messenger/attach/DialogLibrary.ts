// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The chooser of the system (`@tauri-apps/plugin-dialog`; on Android
// ACTION_GET_CONTENT), as the attach button has always used it.

import { MEDIA_EXTENSIONS } from './media';
import { noGrid, type MediaKind, type MediaLibrary } from './MediaLibrary';

/** The options of the system chooser: photos and videos are filtered, a file is not. */
export function openOptions(multiple: boolean, kind: MediaKind) {
  return {
    multiple,
    directory: false,
    filters: kind === 'any' ? undefined : [{ name: 'media', extensions: MEDIA_EXTENSIONS[kind] }],
  };
}

export const dialogLibrary: MediaLibrary = {
  ...noGrid,
  capabilities: { pickSystem: true, grid: false, capture: false, photoPicker: false },
  async pickSystem({ multiple, kind }) {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const picked = await open(openOptions(multiple, kind));
    if (!picked) return [];
    return Array.isArray(picked) ? picked : [picked];
  },
};

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The phone's gallery through the gallery plugin (tauri-plugin-veydan-media,
// reached through the messenger's commands): a grid of the newest photos
// and videos by pages, the albums, thumbnails as addresses of the app's
// server of files, the system camera and the system's Photo Picker. It asks
// for nothing by itself: `access` only reads, `request` is a tap.

import { messengerApi } from '../api';
import { dialogLibrary } from './DialogLibrary';
import type { MediaLibrary } from './MediaLibrary';

export const androidMediaLibrary: MediaLibrary = {
  capabilities: { pickSystem: true, grid: true, capture: true, photoPicker: true },
  access: () => messengerApi.gallery.access(),
  request: () => messengerApi.gallery.request(),
  async recent({ cursor, limit, kind, album }) {
    const k = kind === 'image' || kind === 'video' ? kind : undefined;
    return messengerApi.gallery.recent({ cursor, limit, kind: k, album });
  },
  albums: () => messengerApi.gallery.albums(),
  thumbUrl: (item, px) => messengerApi.gallery.thumb(item.path, Math.round(px)),
  forgetThumbs: () => messengerApi.gallery.forgetThumbs(),
  readable: (paths) => messengerApi.gallery.readable(paths),
  capture: (kind) => messengerApi.gallery.capture(kind),
  async pickSystem({ multiple, kind }) {
    // A file of any kind is the system's chooser of documents, as it has always been.
    if (kind === 'any') return dialogLibrary.pickSystem({ multiple, kind });
    return messengerApi.gallery.pick(multiple, kind);
  },
};

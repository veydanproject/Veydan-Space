// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The source of the browser preview (`make dev`) and of the tests: there
// are no paths in a browser, so it hands over a sample to see the UI with.
// `huge`: a file over the send limit, refused as the app refuses it.
// Its grid is ten thousand drawn tiles, by pages as the phone gives them;
// `?gallery=partial|denied|picker` in the address of the preview shows the
// other states of the access (`picker`: a source without a grid).

import type { Album, MediaAccess, MediaCapabilities, MediaItem, MediaLibrary } from './MediaLibrary';

export const MOCK_PICKED = [
  '/home/dev/Pictures/sample-1.png',
  '/home/dev/Pictures/sample-2.png',
  '/home/dev/Documents/report.pdf',
  '/home/dev/Videos/conference-huge.mov',
];

/** The photos and videos of the preview's gallery. */
export const MOCK_COUNT = 10_000;

const ALBUMS: Album[] = [
  { id: 'camera', title: 'Camera', count: 7_000 },
  { id: 'screenshots', title: 'Screenshots', count: 2_500 },
  { id: 'telegram', title: 'Telegram', count: 500 },
];

/** The item `i` of the preview's gallery: every seventh a video. */
export function mockItem(i: number): MediaItem {
  const video = i % 7 === 3;
  return {
    id: `m${i}`,
    kind: video ? 'video' : 'image',
    path: `/home/dev/Pictures/gallery/${video ? 'clip' : 'photo'}-${i}.${video ? 'mp4' : 'jpg'}`,
    duration: video ? 5 + ((i * 37) % 3600) : undefined,
    width: 1200,
    height: 1600,
  };
}

const albumOf = (i: number) => (i % 20 === 0 ? 'telegram' : i % 4 === 1 ? 'screenshots' : 'camera');

type Mode = 'full' | 'partial' | 'denied' | 'picker';

function mode(): Mode {
  if (typeof location === 'undefined') return 'full';
  const m = new URLSearchParams(location.search).get('gallery');
  return m === 'partial' || m === 'denied' || m === 'picker' ? m : 'full';
}

/** A preview's source of media in one state of the access; the tests make one of each. */
export function makeMockLibrary(state: Mode = mode(), count = MOCK_COUNT): MediaLibrary {
  let access: MediaAccess = state === 'picker' ? 'denied' : state;
  const visible = (i: number) => access === 'full' || (access === 'partial' && i % 50 === 0);
  const capabilities: MediaCapabilities = { pickSystem: true, grid: state !== 'picker', capture: true, photoPicker: true };
  return {
    capabilities,
    access: async () => access,
    async request() {
      if (state !== 'picker') access = 'full';
      return access;
    },
    async recent({ cursor, limit, kind, album }) {
      const items: MediaItem[] = [];
      let i = cursor ? Number(cursor) : 0;
      for (; i < count && items.length < limit; i++) {
        if (!visible(i)) continue;
        const it = mockItem(i);
        if ((kind === 'image' || kind === 'video') && it.kind !== kind) continue;
        if (album && albumOf(i) !== album) continue;
        items.push(it);
      }
      return { items, cursor: i < count ? String(i) : undefined };
    },
    albums: async () => (access === 'denied' ? [] : ALBUMS),
    async thumbUrl(item) {
      const n = Number(item.id.slice(1));
      const hue = (n * 47) % 360;
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 90 90"><rect width="90" height="90" fill="hsl(${hue} 45% 55%)"/><text x="45" y="52" font-size="18" text-anchor="middle" fill="white" font-family="sans-serif">${n}</text></svg>`;
      return `data:image/svg+xml,${encodeURIComponent(svg)}`;
    },
    forgetThumbs: async () => {},
    async readable(paths) {
      return paths.filter((p) => {
        const m = /\/gallery\/(?:photo|clip)-(\d+)\./.exec(p);
        return !!m && visible(Number(m[1]));
      });
    },
    capture: async (kind) => `/home/dev/Pictures/capture-${Date.now()}.${kind === 'video' ? 'mp4' : 'jpg'}`,
    pickSystem: async () => [...MOCK_PICKED],
  };
}

export const mockLibrary: MediaLibrary = makeMockLibrary();

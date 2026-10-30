// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Where the photos and videos to attach come from. The panel of the gallery
// asks what the source can do (`capabilities`), never which source it is.

export type MediaAccess = 'full' | 'partial' | 'denied';
/** `media`: photos and videos; `any`: every file. */
export type MediaKind = 'image' | 'video' | 'media' | 'any';

export interface MediaItem {
  id: string;
  kind: 'image' | 'video';
  /** The path or `content://` address `importPicked` takes. */
  path: string;
  /** Seconds, for a video. */
  duration?: number;
  /** Pixels, when the source knows them. */
  width?: number;
  height?: number;
}

export interface Album {
  id: string;
  title: string;
  count: number;
}

/** What a source can do; a panel offers only that. */
export interface MediaCapabilities {
  /** The system's chooser of files (always, today). */
  pickSystem: boolean;
  /** A grid of the latest photos and videos with their albums. */
  grid: boolean;
  /** Taking a photo or a video with the system camera. */
  capture: boolean;
  /** `pickSystem` of photos and videos is the system's Photo Picker, which needs no access («Выбрать в системной галерее»). */
  photoPicker: boolean;
}

/** What `capture` refuses with when the user did not let the app use the camera. */
export const CAMERA_DENIED = 'camera_denied';

export interface MediaLibrary {
  readonly capabilities: MediaCapabilities;
  access(): Promise<MediaAccess>;
  request(): Promise<MediaAccess>;
  recent(o: { cursor?: string; limit: number; kind?: MediaKind; album?: string }): Promise<{ items: MediaItem[]; cursor?: string }>;
  albums(): Promise<Album[]>;
  /** An http address, never base64. */
  thumbUrl(item: MediaItem, px: number): Promise<string>;
  /** Every address `thumbUrl` gave answers no more (the gallery is read anew: the access may have changed). */
  forgetThumbs(): Promise<void>;
  /**
   * Which of `paths` (items of the grid) the app may still read, each asked
   * of the source by itself, whatever album or page the grid shows.
   */
  readable(paths: string[]): Promise<string[]>;
  /** The path of what the camera took, for `importPicked`; `null`: cancelled. */
  capture(kind: 'photo' | 'video'): Promise<string | null>;
  /** The chooser of the system; the paths picked, none when cancelled. */
  pickSystem(o: { multiple: boolean; kind: MediaKind }): Promise<string[]>;
}

/** A source that cannot show a grid nor take a picture: the three methods say nothing. */
export const noGrid = {
  access: async (): Promise<MediaAccess> => 'denied',
  request: async (): Promise<MediaAccess> => 'denied',
  recent: async () => ({ items: [] as MediaItem[] }),
  albums: async (): Promise<Album[]> => [],
  thumbUrl: async (): Promise<string> => { throw new Error('no grid'); },
  forgetThumbs: async (): Promise<void> => {},
  readable: async (_paths: string[]): Promise<string[]> => [],
  capture: async (): Promise<string | null> => null,
};

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Files picked for a post or a comment wait above the composer, as in a
// chat (ChatWindow's tray), until they are sent with the words as their
// caption: several picked together go as one album (one `batch`, the
// caption with the last), what did not go waits again. A picture shows
// as a small square of itself (`channel_picked_preview`), asked once
// for each path and let go with the tray.

import { videoPoster } from '$lib/messenger/seams/shared';
import { channelsApi, type ChannelPoster, type TextFormat } from './api';

export interface PickedFile {
  key: number;
  /** A path of the disk, or what the picker of a phone gives (`content://`). */
  path: string;
  name: string;
}

/** The name of a picked path, for the tray: `content://…/IMG_1.jpg` too. */
export function pathName(path: string): string {
  const last = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  try { return decodeURIComponent(last); } catch { return last; }
}

/** What says the files went by one action: as the messenger makes it (8 to 64 of `0-9 a-z A-Z - _`). */
export function newBatch(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') return crypto.randomUUID();
  return Array.from({ length: 32 }, () => Math.floor(Math.random() * 16).toString(16)).join('');
}

/** A picture by its name: the tray shows it as one. */
export const isPicture = (name: string) => /\.(png|jpe?g|webp|gif|heic|avif)$/i.test(name);
/** A video by its name. */
export const isVideo = (name: string) => /\.(mp4|m4v|mov|webm|mkv|3gp)$/i.test(name);

/**
 * A file whose kind its name does not tell: what the photo picker of a
 * phone hands over (`content://media/picker/…/media/1000012345`). It may
 * be a video, so it goes the way a video goes (`VideoOf`): the copy of it
 * tells, the frame is taken when it is one.
 */
export const kindUnknown = (path: string, name: string) => path.startsWith('content://') && !/\.[a-z0-9]{2,5}$/i.test(name);

/** The icon of a file the tray has no picture of. */
export const pickedIcon = (name: string) => (isPicture(name) ? 'image' : isVideo(name) ? 'video' : 'file');

/** What a picked file looks like: a URL of a small picture of it, `null` when there is none. */
export type PreviewOf = (path: string) => Promise<string | null>;

/** A video about to go: the file it goes as (a phone's made a file of the device) and the frame its readers see before they fetch it. */
export type VideoOf = (path: string) => Promise<{ path: string; poster: ChannelPoster | null }>;

/** As a chat's composer does: the frame is taken where the webview reads the video (the messenger's server of files). */
export const takeVideo: VideoOf = async (path) => {
  const f = await channelsApi.mediaImport(path);
  return { path: f.path, poster: f.url ? await videoPoster(f.url) : null };
};

/** How a file of the tray goes. */
export interface SendOpts {
  caption?: string;
  /** `md` when the caption is Markdown (the composer's format menu). */
  fmt?: TextFormat | null;
  batch?: string;
  /** A video: the frame its readers see before they fetch it. */
  poster?: ChannelPoster | null;
}

export class Picked {
  files = $state<PickedFile[]>([]);
  /**
   * The picture of a path, once asked: a URL, `null` when it has none
   * (not a picture, one that could not be read or drawn); absent while
   * it is on its way, when the tray shows the icon.
   */
  previews = $state<Record<string, string | null>>({});
  private seq = 0;
  /** The paths asked, each with what tells its answer from that of an earlier ask. */
  private asked = new Map<string, object>();

  constructor(
    private previewOf: PreviewOf = (path) => channelsApi.pickedPreview(path),
    private videoOf: VideoOf = takeVideo,
  ) {}

  add(paths: string[]) {
    this.files = [...this.files, ...paths.map((path) => ({ key: ++this.seq, path, name: pathName(path) }))];
    for (const path of paths) this.ask(path);
  }

  remove(key: number) {
    this.files = this.files.filter((f) => f.key !== key);
    this.prune();
  }

  clear() {
    this.files = [];
    this.prune();
  }

  /** The picture of a path that the page could not draw: the icon instead. */
  failed(path: string) {
    if (this.asked.has(path)) this.set(path, null);
  }

  /** Asked once for each path while it waits in the tray; an answer after it left is let go. */
  private ask(path: string) {
    if (this.asked.has(path)) return;
    const token = {};
    this.asked.set(path, token);
    const current = () => this.asked.get(path) === token;
    this.previewOf(path).then(
      (url) => {
        if (current()) this.set(path, url);
        else drop(url);
      },
      () => { if (current()) this.set(path, null); },
    );
  }

  private set(path: string, url: string | null) {
    drop(this.previews[path]);
    this.previews = { ...this.previews, [path]: url };
  }

  /** The pictures of paths no longer in the tray go, with what holds them. */
  private prune() {
    const kept = new Set(this.files.map((f) => f.path));
    let gone = false;
    for (const path of [...this.asked.keys()]) {
      if (kept.has(path)) continue;
      this.asked.delete(path);
      if (path in this.previews) { drop(this.previews[path]); gone = true; }
    }
    if (gone) this.previews = Object.fromEntries(Object.entries(this.previews).filter(([p]) => kept.has(p)));
  }

  /**
   * Sends what waits, in the order picked: several as one album, the
   * caption with the last (`fmt`: how it is written). A file refused stops
   * the rest; it and those after it wait again, and the refusal is thrown.
   */
  async send(go: (path: string, opts: SendOpts) => Promise<unknown>, caption = '', fmt: TextFormat | null = null) {
    const files = this.files;
    this.files = [];
    const batch = files.length > 1 ? newBatch() : undefined;
    for (const [i, f] of files.entries()) {
      try {
        const last = i === files.length - 1 && !!caption;
        const opts: SendOpts = { caption: last ? caption : undefined, fmt: last && fmt ? fmt : undefined, batch };
        // A video goes with a frame of it, as in a chat; without one when it cannot be taken. A phone's
        // file of no known kind goes as the copy the import makes (a video among them with its frame).
        const video = isVideo(f.name) || kindUnknown(f.path, f.name) ? await this.videoOf(f.path).catch(() => null) : null;
        await (video ? go(video.path, { ...opts, poster: video.poster }) : go(f.path, opts));
      } catch (e) {
        this.files = [...files.slice(i), ...this.files];
        throw e;
      }
    }
    // All of it went: its pictures go too (what failed keeps them, it waits again).
    this.prune();
  }
}

/** A `blob:` URL holds its bytes until it is revoked. */
function drop(url: string | null | undefined) {
  if (url?.startsWith('blob:')) URL.revokeObjectURL(url);
}

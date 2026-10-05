// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The host hands an attachment over as a `data:` url. The policy of the
// page (`media-src`) lets a picture come from `data:`, but sound and video
// only from `blob:`; so what is played is turned into a blob first.

/** The bytes of a base64 `data:` url, with its type; `null` when it is not one. */
export function dataUrlToBlob(dataUrl: string): Blob | null {
  const comma = dataUrl.indexOf(',');
  if (!dataUrl.startsWith('data:') || comma < 0) return null;
  const head = dataUrl.slice(5, comma);
  if (!head.endsWith(';base64')) return null;
  let bin: string;
  try { bin = atob(dataUrl.slice(comma + 1)); } catch { return null; }
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new Blob([bytes], { type: head.slice(0, -7) });
}

/** A `blob:` url of the same bytes; the caller revokes it. The url itself when it cannot be read. */
export function playableUrl(dataUrl: string): string {
  const blob = dataUrlToBlob(dataUrl);
  return blob ? URL.createObjectURL(blob) : dataUrl;
}

/** One transparent pixel: without a poster Android draws its own placeholder over a video. */
export const NO_POSTER = 'data:image/gif;base64,R0lGODlhAQABAAAAACH5BAEKAAEALAAAAAABAAEAAAICTAEAOw==';

/**
 * A page draws nothing of a video that never played: play it muted for a
 * moment so its first frame is shown. `busy` says the user already plays it.
 */
export function showFirstFrame(video: HTMLVideoElement, busy: () => boolean) {
  if (busy()) return;
  video.muted = true;
  video.play()
    .then(() => { if (!busy()) { video.pause(); video.currentTime = 0; } })
    .catch(() => {})
    .finally(() => { if (!busy()) video.muted = false; });
}
